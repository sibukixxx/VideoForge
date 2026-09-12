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
pub use live2d::Live2dModelInfo;

/// VOICEVOX only, for now (design §5). Kept as a string rather than an enum
/// so a future provider does not require a schema break.
pub const PROVIDER_VOICEVOX: &str = "voicevox";
/// Live2D only, for now (design §14).
pub const MODEL_TYPE_LIVE2D: &str = "live2d";

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
    #[error("Live2D model file not found: {path}")]
    ModelNotFound { path: PathBuf },
    #[error("Live2D model file {path} is not valid model3.json: {reason}")]
    ModelInvalid { path: PathBuf, reason: String },
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
/// repository and outside any VideoForge workspace (design §6): `path` is
/// whatever the user supplied and may be absolute, or relative to the
/// manifest file itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CharacterModel {
    #[serde(rename = "type", default = "default_model_type")]
    pub model_type: String,
    pub path: String,
}

fn default_model_type() -> String {
    MODEL_TYPE_LIVE2D.to_string()
}

impl CharacterModel {
    /// Resolve `path` against the manifest file's directory when it is not
    /// already absolute. Mirrors how a lockfile resolves sibling paths — the
    /// model does not have to live next to the manifest, but a relative path
    /// is convenient when both are kept in a personal asset folder.
    pub fn resolve_path(&self, manifest_dir: &Path) -> PathBuf {
        let p = Path::new(&self.path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            manifest_dir.join(p)
        }
    }
}

/// One character: metadata only, never a bespoke per-character type (design §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// A standalone, reusable manifest of characters (design §5). Deliberately
/// ignorant of any one VideoForge project: `videoforge.yaml` links to a
/// manifest by path and a speaker by `character_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
                if model.model_type != MODEL_TYPE_LIVE2D {
                    return Err(CharacterError::UnknownModelType {
                        id: c.id.clone(),
                        model_type: model.model_type.clone(),
                    });
                }
                if model.path.trim().is_empty() {
                    return Err(CharacterError::EmptyModelPath { id: c.id.clone() });
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

    #[test]
    fn model_path_resolves_relative_to_manifest_dir() {
        let model = CharacterModel {
            model_type: MODEL_TYPE_LIVE2D.into(),
            path: "sub/model3.json".into(),
        };
        let dir = Path::new("/home/user/characters");
        assert_eq!(
            model.resolve_path(dir),
            Path::new("/home/user/characters/sub/model3.json")
        );
        let abs = CharacterModel {
            model_type: MODEL_TYPE_LIVE2D.into(),
            path: "/models/tsumugi/model3.json".into(),
        };
        assert_eq!(
            abs.resolve_path(dir),
            Path::new("/models/tsumugi/model3.json")
        );
    }
}
