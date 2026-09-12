//! Character manifest domain model (VOICEVOX + Live2D character video
//! pipeline; see `docs/character-system.md`).
//!
//! A [`CharacterManifest`] is a small, standalone, reusable YAML file that
//! describes *who* a character is (display name, VOICEVOX voice identity,
//! optional Live2D model) without saying anything about a specific video
//! project. It intentionally knows nothing about scripts, timelines, or
//! VideoForge workspaces — `videoforge-core` links a project's speakers to
//! entries here by id (`SpeakerConfig::character_id`).
//!
//! No character is special-cased: a character is metadata (`id`,
//! `display_name`, an optional `voice` reference resolved by *name* rather
//! than a hard-coded numeric id, and an optional Live2D `model` reference).
//! Live2D model files themselves are never part of this repository or its
//! releases (see `docs/character-licensing.md`); a manifest only points at a
//! path the user supplies locally.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod live2d;
pub mod png;
pub mod png_lipsync;
pub use live2d::Live2dModelInfo;
pub use png_lipsync::PngLipsyncAssets;

/// VOICEVOX only, for now (design §5). Kept as a string rather than an enum
/// so a future provider does not require a schema break.
pub const PROVIDER_VOICEVOX: &str = "voicevox";
/// A Live2D Cubism model, driven by a (not yet implemented, see
/// `docs/live2d-renderer-decision.md`) headless-browser renderer.
pub const MODEL_TYPE_LIVE2D: &str = "live2d";
/// Three static, transparent PNGs (closed / half / open mouth) swapped by
/// amplitude, composited directly by `videoforge-preview`'s FFmpeg pipeline
/// (P0-1). No external renderer or license question, unlike Live2D.
pub const MODEL_TYPE_PNG_LIPSYNC: &str = "png_lipsync";

/// `CharacterPresentation::scale` bounds (design: "thresholdはhard-codeする
/// 場合でも定数化する"). Not engine limits — they exist so a typo in a
/// character manifest (`scale: 20` instead of `scale: 0.2`) fails at load
/// time instead of producing a character that fills (or vanishes from) the
/// frame.
pub const MIN_CHARACTER_SCALE: f32 = 0.1;
pub const MAX_CHARACTER_SCALE: f32 = 3.0;

#[derive(Debug, Error)]
pub enum CharacterError {
    #[error("failed to read character manifest {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid character manifest {path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("character manifest has no `characters` entries")]
    Empty,
    #[error("a character id must not be empty")]
    EmptyId,
    #[error("character id `{id}` is declared more than once")]
    DuplicateId { id: String },
    #[error(
        "character `{id}` has unknown voice provider `{provider}` (only `{PROVIDER_VOICEVOX}` is supported)"
    )]
    UnknownVoiceProvider { id: String, provider: String },
    #[error(
        "character `{id}` has unknown model type `{model_type}` (only `{MODEL_TYPE_LIVE2D}` is supported)"
    )]
    UnknownModelType { id: String, model_type: String },
    #[error("character `{id}` voice.speaker must not be empty")]
    EmptyVoiceSpeaker { id: String },
    #[error("character `{id}` voice.style must not be empty")]
    EmptyVoiceStyle { id: String },
    #[error("character `{id}` model.path must not be empty")]
    EmptyModelPath { id: String },
    #[error("character `{id}` model.{field} must not be empty (required for `{MODEL_TYPE_PNG_LIPSYNC}`)")]
    EmptyPngField { id: String, field: &'static str },
    #[error("Live2D model file not found: {path}")]
    ModelNotFound { path: PathBuf },
    #[error("Live2D model file {path} is not valid model3.json: {reason}")]
    ModelInvalid { path: PathBuf, reason: String },
    #[error("PNG sprite not found: {path}")]
    PngNotFound { path: PathBuf },
    #[error("PNG sprite {path} is invalid: {reason}")]
    PngInvalid { path: PathBuf, reason: String },
    #[error(
        "PNG sprite {path} has no alpha channel (must be exported as RGBA or grayscale+alpha)"
    )]
    PngNotTransparent { path: PathBuf },
    #[error(
        "character `{id}` presentation.scale must be between {MIN_CHARACTER_SCALE} and {MAX_CHARACTER_SCALE} (got {scale})"
    )]
    InvalidScale { id: String, scale: f32 },
}

/// A VOICEVOX voice identity, resolved by *name* rather than a hard-coded
/// numeric id (design §5, §21) — `videoforge-core` turns this into a
/// `speaker_id` via `TtsEngine::list_speakers()` at generate/doctor time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterVoice {
    #[serde(default = "default_provider")]
    pub provider: String,
    /// VOICEVOX speaker display name, e.g. `"春日部つむぎ"`.
    pub speaker: String,
    /// VOICEVOX style name, e.g. `"ノーマル"`.
    pub style: String,
}

fn default_provider() -> String {
    PROVIDER_VOICEVOX.to_string()
}

/// A character model reference. The model file itself lives outside the
/// repository and outside any VideoForge workspace (design §6): every path
/// field is whatever the user supplied and may be absolute, or relative to
/// the manifest file itself.
///
/// Exactly one shape is populated depending on `model_type`: `path` for
/// `live2d`, `closed`/`half`/`open` for `png_lipsync`
/// (`CharacterManifest::validate` enforces this — the fields are all
/// `Option` here only because the two model types need different ones, not
/// because any of them is optional *within* its own type).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CharacterModel {
    #[serde(rename = "type", default = "default_model_type")]
    pub model_type: String,
    /// `live2d`: path to the model's `model3.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `png_lipsync`: closed-mouth sprite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    /// `png_lipsync`: half-open-mouth sprite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<String>,
    /// `png_lipsync`: fully-open-mouth sprite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
}

fn default_model_type() -> String {
    MODEL_TYPE_LIVE2D.to_string()
}

/// Resolve a manifest-relative (or absolute) path string against the
/// manifest file's directory. Mirrors how a lockfile resolves sibling paths.
fn resolve_manifest_path(value: &str, manifest_dir: &Path) -> PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        manifest_dir.join(p)
    }
}

impl CharacterModel {
    /// Resolve the `live2d` model path. Panics-free even on a `png_lipsync`
    /// model (returns an empty-looking path) — `CharacterManifest::validate`
    /// is what guarantees callers only reach this for a `live2d` model.
    pub fn resolve_path(&self, manifest_dir: &Path) -> PathBuf {
        resolve_manifest_path(self.path.as_deref().unwrap_or_default(), manifest_dir)
    }

    fn resolve_required(
        &self,
        field: &'static str,
        value: &Option<String>,
        manifest_dir: &Path,
    ) -> Result<PathBuf, CharacterError> {
        let value = value
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| CharacterError::EmptyPngField {
                id: String::new(),
                field,
            })?;
        Ok(resolve_manifest_path(value, manifest_dir))
    }

    /// Resolve the `png_lipsync` closed-mouth sprite path.
    pub fn resolve_closed(&self, manifest_dir: &Path) -> Result<PathBuf, CharacterError> {
        self.resolve_required("closed", &self.closed, manifest_dir)
    }

    /// Resolve the `png_lipsync` half-open-mouth sprite path.
    pub fn resolve_half(&self, manifest_dir: &Path) -> Result<PathBuf, CharacterError> {
        self.resolve_required("half", &self.half, manifest_dir)
    }

    /// Resolve the `png_lipsync` fully-open-mouth sprite path.
    pub fn resolve_open(&self, manifest_dir: &Path) -> Result<PathBuf, CharacterError> {
        self.resolve_required("open", &self.open, manifest_dir)
    }

    pub fn is_live2d(&self) -> bool {
        self.model_type == MODEL_TYPE_LIVE2D
    }

    pub fn is_png_lipsync(&self) -> bool {
        self.model_type == MODEL_TYPE_PNG_LIPSYNC
    }
}

/// Horizontal placement of a character's sprite in the frame. A closed set
/// on purpose, same reasoning as `videoforge_project::FitMode`: an unknown
/// value is a manifest error (via `serde`'s own "unknown variant" message),
/// not a silently-ignored default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterPosition {
    Left,
    #[default]
    Center,
    Right,
}

fn default_scale() -> f32 {
    1.0
}

/// Where and how large a character's sprite appears on screen (P0-1). Kept
/// deliberately small — horizontal slot plus a size multiplier — matching
/// the "2人以上のCharacterを配置可能" / "frame外にはみ出さない" requirements
/// without inventing a second transform system: `videoforge-core` maps this
/// onto the exact same `Transform` every other visual clip already uses.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterPresentation {
    #[serde(default)]
    pub position: CharacterPosition,
    #[serde(default = "default_scale")]
    pub scale: f32,
}

impl Default for CharacterPresentation {
    fn default() -> Self {
        Self {
            position: CharacterPosition::default(),
            scale: default_scale(),
        }
    }
}

/// One character: metadata only, never a bespoke per-character type (design §4).
///
/// `PartialEq` only, not `Eq`: `presentation.scale` is an `f32`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<CharacterVoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<CharacterModel>,
    /// Explicit expression allow-list. When empty and `model` is set, the
    /// list is instead read from the model's `model3.json`.
    #[serde(default)]
    pub expressions: Vec<String>,
    /// Explicit motion allow-list; same fallback as `expressions`.
    #[serde(default)]
    pub motions: Vec<String>,
    /// On-screen placement for a `png_lipsync` model (P0-1). `None` uses
    /// `CharacterPresentation::default()` (centered, unscaled) — meaningless
    /// for a `live2d` model today since nothing renders its frames yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<CharacterPresentation>,
}

/// A standalone, reusable manifest of characters (design §5). Deliberately
/// ignorant of any one VideoForge project: `videoforge.yaml` links to a
/// manifest by path and a speaker by `character_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CharacterManifest {
    #[serde(default)]
    pub characters: Vec<Character>,
}

impl CharacterManifest {
    pub fn parse(yaml: &str, path: &Path) -> Result<Self, CharacterError> {
        let manifest: CharacterManifest =
            serde_yaml::from_str(yaml).map_err(|e| CharacterError::Yaml {
                path: path.to_path_buf(),
                source: e,
            })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn load(path: &Path) -> Result<Self, CharacterError> {
        let text = std::fs::read_to_string(path).map_err(|e| CharacterError::Read {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::parse(&text, path)
    }

    /// Structural checks that need no filesystem or network access: ids are
    /// non-empty and unique, provider/model type are ones we know about, and
    /// referenced fields are non-empty. Live2D model *file* validity is
    /// checked separately by [`live2d::load_model3_json`] since it needs the
    /// manifest's directory to resolve a relative `model.path`.
    pub fn validate(&self) -> Result<(), CharacterError> {
        if self.characters.is_empty() {
            return Err(CharacterError::Empty);
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for c in &self.characters {
            if c.id.trim().is_empty() {
                return Err(CharacterError::EmptyId);
            }
            if !seen.insert(c.id.as_str()) {
                return Err(CharacterError::DuplicateId { id: c.id.clone() });
            }
            if let Some(voice) = &c.voice {
                if voice.provider != PROVIDER_VOICEVOX {
                    return Err(CharacterError::UnknownVoiceProvider {
                        id: c.id.clone(),
                        provider: voice.provider.clone(),
                    });
                }
                if voice.speaker.trim().is_empty() {
                    return Err(CharacterError::EmptyVoiceSpeaker { id: c.id.clone() });
                }
                if voice.style.trim().is_empty() {
                    return Err(CharacterError::EmptyVoiceStyle { id: c.id.clone() });
                }
            }
            if let Some(model) = &c.model {
                match model.model_type.as_str() {
                    MODEL_TYPE_LIVE2D => {
                        let empty = model.path.as_deref().unwrap_or_default().trim().is_empty();
                        if empty {
                            return Err(CharacterError::EmptyModelPath { id: c.id.clone() });
                        }
                    }
                    MODEL_TYPE_PNG_LIPSYNC => {
                        for (field, value) in [
                            ("closed", &model.closed),
                            ("half", &model.half),
                            ("open", &model.open),
                        ] {
                            let empty = value.as_deref().unwrap_or_default().trim().is_empty();
                            if empty {
                                return Err(CharacterError::EmptyPngField {
                                    id: c.id.clone(),
                                    field,
                                });
                            }
                        }
                    }
                    other => {
                        return Err(CharacterError::UnknownModelType {
                            id: c.id.clone(),
                            model_type: other.to_string(),
                        })
                    }
                }
            }
            if let Some(presentation) = &c.presentation {
                if !presentation.scale.is_finite()
                    || presentation.scale < MIN_CHARACTER_SCALE
                    || presentation.scale > MAX_CHARACTER_SCALE
                {
                    return Err(CharacterError::InvalidScale {
                        id: c.id.clone(),
                        scale: presentation.scale,
                    });
                }
            }
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&Character> {
        self.characters.iter().find(|c| c.id == id)
    }

    pub fn character_ids(&self) -> Vec<String> {
        self.characters.iter().map(|c| c.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
characters:
  - id: tsumugi
    display_name: "春日部つむぎ"
    voice:
      provider: voicevox
      speaker: "春日部つむぎ"
      style: "ノーマル"
    model:
      type: live2d
      path: "./model3.json"
"#
    }

    #[test]
    fn parses_valid_manifest() {
        let m = CharacterManifest::parse(sample_yaml(), Path::new("characters.yaml")).unwrap();
        assert_eq!(m.characters.len(), 1);
        let c = m.find("tsumugi").unwrap();
        assert_eq!(c.display_name, "春日部つむぎ");
        assert_eq!(c.voice.as_ref().unwrap().speaker, "春日部つむぎ");
        assert_eq!(c.model.as_ref().unwrap().model_type, "live2d");
    }

    #[test]
    fn character_without_voice_or_model_is_valid() {
        let yaml = "characters:\n  - id: narrator\n    display_name: Narrator\n";
        let m = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap();
        assert_eq!(m.find("narrator").unwrap().voice, None);
    }

    #[test]
    fn rejects_empty_manifest() {
        let err = CharacterManifest::parse("characters: []\n", Path::new("x.yaml")).unwrap_err();
        assert!(matches!(err, CharacterError::Empty));
    }

    #[test]
    fn rejects_duplicate_ids() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n  - id: a\n    display_name: A2\n";
        let err = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap_err();
        assert!(matches!(err, CharacterError::DuplicateId { id } if id == "a"));
    }

    #[test]
    fn rejects_unknown_voice_provider() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n    voice:\n      provider: elevenlabs\n      speaker: x\n      style: y\n";
        let err = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap_err();
        assert!(matches!(err, CharacterError::UnknownVoiceProvider { .. }));
    }

    #[test]
    fn rejects_unknown_model_type() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n    model:\n      type: spine\n      path: x\n";
        let err = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap_err();
        assert!(matches!(err, CharacterError::UnknownModelType { .. }));
    }

    #[test]
    fn rejects_unknown_fields_like_config_does() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n    nickname: nope\n";
        assert!(CharacterManifest::parse(yaml, Path::new("x.yaml")).is_err());
    }

    fn live2d_model(path: &str) -> CharacterModel {
        CharacterModel {
            model_type: MODEL_TYPE_LIVE2D.into(),
            path: Some(path.into()),
            closed: None,
            half: None,
            open: None,
        }
    }

    #[test]
    fn model_path_resolves_relative_to_manifest_dir() {
        let model = live2d_model("sub/model3.json");
        let dir = Path::new("/home/user/characters");
        assert_eq!(
            model.resolve_path(dir),
            Path::new("/home/user/characters/sub/model3.json")
        );
        let abs = live2d_model("/models/tsumugi/model3.json");
        assert_eq!(
            abs.resolve_path(dir),
            Path::new("/models/tsumugi/model3.json")
        );
    }

    fn png_lipsync_yaml(closed: &str, half: &str, open: &str) -> String {
        format!(
            "characters:\n  - id: a\n    display_name: A\n    model:\n      type: png_lipsync\n      closed: {closed}\n      half: {half}\n      open: {open}\n"
        )
    }

    #[test]
    fn parses_png_lipsync_model() {
        let yaml = png_lipsync_yaml("c.png", "h.png", "o.png");
        let m = CharacterManifest::parse(&yaml, Path::new("x.yaml")).unwrap();
        let model = m.find("a").unwrap().model.as_ref().unwrap();
        assert!(model.is_png_lipsync());
        assert_eq!(model.closed.as_deref(), Some("c.png"));
        let dir = Path::new("/base");
        assert_eq!(model.resolve_closed(dir).unwrap(), Path::new("/base/c.png"));
        assert_eq!(model.resolve_half(dir).unwrap(), Path::new("/base/h.png"));
        assert_eq!(model.resolve_open(dir).unwrap(), Path::new("/base/o.png"));
    }

    #[test]
    fn rejects_png_lipsync_missing_sprite_field() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n    model:\n      type: png_lipsync\n      closed: c.png\n      half: h.png\n";
        let err = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap_err();
        assert!(matches!(
            err,
            CharacterError::EmptyPngField { field: "open", .. }
        ));
    }

    #[test]
    fn rejects_out_of_range_presentation_scale() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n    presentation:\n      position: left\n      scale: 10.0\n";
        let err = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap_err();
        assert!(matches!(err, CharacterError::InvalidScale { .. }));
    }

    #[test]
    fn rejects_unknown_presentation_position() {
        let yaml =
            "characters:\n  - id: a\n    display_name: A\n    presentation:\n      position: up\n";
        assert!(CharacterManifest::parse(yaml, Path::new("x.yaml")).is_err());
    }

    #[test]
    fn presentation_defaults_to_centered_unscaled() {
        let yaml = "characters:\n  - id: a\n    display_name: A\n";
        let m = CharacterManifest::parse(yaml, Path::new("x.yaml")).unwrap();
        assert_eq!(m.find("a").unwrap().presentation, None);
        assert_eq!(
            CharacterPresentation::default().position,
            CharacterPosition::Center
        );
        assert_eq!(CharacterPresentation::default().scale, 1.0);
    }
}
