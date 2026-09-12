//! Links a project's speakers to entries in a standalone character manifest
//! (design §5) and resolves VOICEVOX voices by *name* instead of a
//! hard-coded numeric id.
//!
//! This module is the only place that turns `SpeakerConfig::character_id`
//! into anything: everything downstream (TTS synthesis, the cache key,
//! `videoforge-timeline`) keeps working with the plain numeric
//! `VoiceParams::speaker_id` it always has, so a workspace that never sets
//! `character_id` is completely unaffected by this feature existing.

use std::path::{Path, PathBuf};

use videoforge_character::{Character, CharacterManifest, CharacterPosition};
use videoforge_project::Transform;

use crate::config::Config;
use crate::error::AppError;
use crate::tts::{Speaker, TtsEngine};
use crate::workspace::Workspace;

// Horizontal slot a `png_lipsync` character's `presentation.position` maps
// onto, in the same normalized-centre coordinate system every other visual
// clip's `Transform` already uses (`videoforge_project::Transform`). Hard-
// coded for P0 (design: "thresholdはhard-codeする場合でも定数化する") —
// chosen so two characters at `left`/`right` sit clear of both the frame
// edge and each other.
pub const CHARACTER_POSITION_X_LEFT: f32 = 0.20;
pub const CHARACTER_POSITION_X_CENTER: f32 = 0.5;
pub const CHARACTER_POSITION_X_RIGHT: f32 = 0.80;
/// Default vertical centre for a character sprite: low enough to read as
/// "standing", clamped further at render time (`videoforge-preview`) to
/// never overlap the caption safe area.
pub const CHARACTER_POSITION_Y: f32 = 0.80;

/// Map a character's declared `presentation` (or its default, when unset)
/// onto the one placement concept every visual clip already uses. This is
/// the *only* place that interprets `CharacterPosition`; both
/// `core::generate` (writing `project.vfp.json`) and any future consumer of
/// the project IR see a plain `Transform`, not the character crate's enum.
pub fn presentation_transform(character: &Character) -> Transform {
    let presentation = character.presentation.unwrap_or_default();
    let x = match presentation.position {
        CharacterPosition::Left => CHARACTER_POSITION_X_LEFT,
        CharacterPosition::Center => CHARACTER_POSITION_X_CENTER,
        CharacterPosition::Right => CHARACTER_POSITION_X_RIGHT,
    };
    Transform {
        x,
        y: CHARACTER_POSITION_Y,
        scale: presentation.scale,
        ..Transform::default()
    }
}

/// Absolute, resolved paths to a `png_lipsync` character's three sprites —
/// resolved the same way as a Live2D `model.path` (design §6: outside the
/// project IR, outside the workspace, never copied into `generated/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPngSprites {
    pub closed: PathBuf,
    pub half: PathBuf,
    pub open: PathBuf,
}

/// Resolve a character's PNG sprites, if it has a `png_lipsync` model.
/// `Ok(None)` for a voice-only character, a Live2D character (frame
/// rendering is still Phase 1), or a character with no model at all.
pub fn resolve_png_sprites(
    character: &Character,
    manifest_dir: &Path,
) -> Result<Option<ResolvedPngSprites>, AppError> {
    let Some(model) = &character.model else {
        return Ok(None);
    };
    if !model.is_png_lipsync() {
        return Ok(None);
    }
    Ok(Some(ResolvedPngSprites {
        closed: model.resolve_closed(manifest_dir)?,
        half: model.resolve_half(manifest_dir)?,
        open: model.resolve_open(manifest_dir)?,
    }))
}

/// A loaded character manifest plus the path it was loaded from, so callers
/// that need to resolve a character's (possibly relative) model path know
/// what directory to resolve it against.
pub struct LoadedManifest {
    pub manifest: CharacterManifest,
    pub path: PathBuf,
}

/// Load the character manifest a workspace's config points at. Returns
/// `Ok(None)` when no speaker links a character — the common case for a
/// project that does not use this feature at all, and a hard no-op with no
/// filesystem access.
pub fn load_manifest(
    config: &Config,
    workspace: &Workspace,
) -> Result<Option<LoadedManifest>, AppError> {
    if !config.speakers.values().any(|s| s.character_id.is_some()) {
        return Ok(None);
    }
    // `Config::validate` already rejects character_id without a manifest
    // path, but this function is also used by callers (like `doctor`) that
    // may hold an unvalidated config; fail the same way here rather than
    // panicking.
    let rel = config
        .character_manifest
        .as_deref()
        .ok_or_else(|| AppError::InvalidConfig {
            path: workspace.root().join("videoforge.yaml"),
            reason: "a speaker sets character_id but no character_manifest is configured".into(),
        })?;
    let path = workspace.resolve(rel)?;
    let manifest = CharacterManifest::load(&path)?;
    Ok(Some(LoadedManifest { manifest, path }))
}

/// Fill in `SpeakerConfig::voice.speaker_id` for every speaker whose linked
/// character declares a VOICEVOX voice by name, using `tts.list_speakers()`.
/// A no-op — no network call at all — when no speaker links a character with
/// a voice, so existing numeric-id configs are completely unaffected.
pub async fn resolve_character_voices(
    config: &mut Config,
    workspace: &Workspace,
    tts: &dyn TtsEngine,
) -> Result<(), AppError> {
    let Some(loaded) = load_manifest(config, workspace)? else {
        return Ok(());
    };
    let manifest = &loaded.manifest;
    let keys: Vec<String> = config
        .speakers
        .iter()
        .filter(|(_, s)| s.character_id.is_some())
        .map(|(k, _)| k.clone())
        .collect();

    let mut speakers_cache: Option<Vec<Speaker>> = None;
    for key in keys {
        let character_id = config.speakers[&key]
            .character_id
            .clone()
            .expect("filtered above");
        let character =
            manifest
                .find(&character_id)
                .ok_or_else(|| AppError::CharacterNotFound {
                    id: character_id.clone(),
                    known: manifest.character_ids().join(", "),
                })?;
        let Some(voice) = &character.voice else {
            continue;
        };
        let speakers = match &speakers_cache {
            Some(s) => s,
            None => {
                speakers_cache = Some(tts.list_speakers().await?);
                speakers_cache.as_ref().expect("just inserted")
            }
        };
        let id =
            resolve_speaker_style_id(speakers, &voice.speaker, &voice.style).ok_or_else(|| {
                AppError::VoicevoxSpeakerNotFound {
                    speaker: voice.speaker.clone(),
                    style: voice.style.clone(),
                    known: describe_speakers(speakers),
                }
            })?;
        config
            .speakers
            .get_mut(&key)
            .expect("key came from config.speakers")
            .voice
            .speaker_id = id;
    }
    Ok(())
}

fn resolve_speaker_style_id(speakers: &[Speaker], name: &str, style: &str) -> Option<u32> {
    speakers
        .iter()
        .find(|s| s.name == name)
        .and_then(|s| s.styles.iter().find(|st| st.name == style))
        .map(|st| st.id)
}

fn describe_speakers(speakers: &[Speaker]) -> String {
    speakers
        .iter()
        .map(|s| {
            let styles: Vec<&str> = s.styles.iter().map(|st| st.name.as_str()).collect();
            format!("{} ({})", s.name, styles.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use crate::tts::FakeTtsEngine;

    /// A complete, minimal `videoforge.yaml` body (must include its own
    /// `speakers:` block) parsed directly — deliberately not layered on top
    /// of `default_config_yaml`, whose own `speakers:` block would otherwise
    /// collide with the one under test.
    fn workspace_with_config(yaml: &str) -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(yaml, dir.path()).unwrap();
        (dir, ws, cfg)
    }

    #[test]
    fn no_op_when_no_speaker_links_a_character() {
        let (_dir, ws, cfg) =
            workspace_with_config("speakers:\n  a:\n    voice:\n      speaker_id: 1\n");
        assert!(load_manifest(&cfg, &ws).unwrap().is_none());
    }

    #[tokio::test]
    async fn resolves_named_voice_from_manifest() {
        let (dir, ws, mut cfg) = workspace_with_config(
            "character_manifest: characters.yaml\nspeakers:\n  tsumugi:\n    character_id: mock\n",
        );
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/manifest.yaml"
            ),
            dir.path().join("characters.yaml"),
        )
        .unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/model3.json"
            ),
            dir.path().join("model3.json"),
        )
        .unwrap();

        let tts = FakeTtsEngine::default();
        resolve_character_voices(&mut cfg, &ws, &tts).await.unwrap();
        // FakeTtsEngine::list_speakers() always returns Speaker{name:"Fake", styles:[{id:0,name:"silence"}]},
        // matching the mock manifest's voice{speaker:"Fake", style:"silence"}.
        assert_eq!(cfg.speakers["tsumugi"].voice.speaker_id, 0);
    }

    #[tokio::test]
    async fn unknown_character_id_is_an_error() {
        let (dir, ws, mut cfg) = workspace_with_config(
            "character_manifest: characters.yaml\nspeakers:\n  tsumugi:\n    character_id: nope\n",
        );
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/manifest.yaml"
            ),
            dir.path().join("characters.yaml"),
        )
        .unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/model3.json"
            ),
            dir.path().join("model3.json"),
        )
        .unwrap();

        let tts = FakeTtsEngine::default();
        let err = resolve_character_voices(&mut cfg, &ws, &tts)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "character_not_found");
    }

    fn png_character(position: Option<&str>, scale: Option<f32>) -> Character {
        use videoforge_character::{CharacterModel, CharacterPresentation};
        let presentation = position.map(|p| {
            let position = match p {
                "left" => videoforge_character::CharacterPosition::Left,
                "right" => videoforge_character::CharacterPosition::Right,
                _ => videoforge_character::CharacterPosition::Center,
            };
            CharacterPresentation {
                position,
                scale: scale.unwrap_or(1.0),
            }
        });
        Character {
            id: "a".into(),
            display_name: "A".into(),
            voice: None,
            model: Some(CharacterModel {
                model_type: videoforge_character::MODEL_TYPE_PNG_LIPSYNC.into(),
                path: None,
                closed: Some("closed.png".into()),
                half: Some("half.png".into()),
                open: Some("open.png".into()),
            }),
            expressions: Vec::new(),
            motions: Vec::new(),
            presentation,
        }
    }

    #[test]
    fn presentation_transform_maps_position_to_x() {
        let left = presentation_transform(&png_character(Some("left"), Some(0.5)));
        assert_eq!(left.x, CHARACTER_POSITION_X_LEFT);
        assert_eq!(left.scale, 0.5);

        let right = presentation_transform(&png_character(Some("right"), None));
        assert_eq!(right.x, CHARACTER_POSITION_X_RIGHT);
        assert_eq!(right.scale, 1.0);

        let default = presentation_transform(&png_character(None, None));
        assert_eq!(default.x, CHARACTER_POSITION_X_CENTER);
    }

    #[test]
    fn resolve_png_sprites_resolves_all_three_paths() {
        let c = png_character(Some("left"), None);
        let dir = Path::new("/base");
        let sprites = resolve_png_sprites(&c, dir).unwrap().unwrap();
        assert_eq!(sprites.closed, Path::new("/base/closed.png"));
        assert_eq!(sprites.half, Path::new("/base/half.png"));
        assert_eq!(sprites.open, Path::new("/base/open.png"));
    }

    #[test]
    fn resolve_png_sprites_is_none_for_voice_only_character() {
        let c = Character {
            id: "b".into(),
            display_name: "B".into(),
            voice: None,
            model: None,
            expressions: Vec::new(),
            motions: Vec::new(),
            presentation: None,
        };
        assert!(resolve_png_sprites(&c, Path::new("/base"))
            .unwrap()
            .is_none());
    }
}
