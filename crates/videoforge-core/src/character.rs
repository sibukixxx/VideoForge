//! Links a project's speakers to entries in a standalone character manifest
//! (design §5) and resolves VOICEVOX voices by *name* instead of a
//! hard-coded numeric id.
//!
//! This module is the only place that turns `SpeakerConfig::character_id`
//! into anything: everything downstream (TTS synthesis, the cache key,
//! `videoforge-timeline`) keeps working with the plain numeric
//! `VoiceParams::speaker_id` it always has.

use std::path::{Path, PathBuf};

use videoforge_character::{Character, CharacterManifest, CharacterPosition};
use videoforge_project::Transform;

use crate::config::Config;
use crate::error::AppError;
use crate::tts::{Speaker, TtsEngine};
use crate::workspace::Workspace;

pub const CHARACTER_POSITION_X_LEFT: f32 = 0.20;
pub const CHARACTER_POSITION_X_CENTER: f32 = 0.5;
pub const CHARACTER_POSITION_X_RIGHT: f32 = 0.80;
pub const CHARACTER_POSITION_Y: f32 = 0.80;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPngSprites {
    pub closed: PathBuf,
    pub half: PathBuf,
    pub open: PathBuf,
}

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

pub struct LoadedManifest {
    pub manifest: CharacterManifest,
    pub path: PathBuf,
}

pub fn load_manifest(
    config: &Config,
    workspace: &Workspace,
) -> Result<Option<LoadedManifest>, AppError> {
    if !config.speakers.values().any(|s| s.character_id.is_some()) {
        return Ok(None);
    }
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

/// Resolve character-linked named voices and, for a real TTS engine, enforce
/// issue #46 speaker-profile consistency before generation proceeds.
///
/// The synthetic `fake` engine is deliberately exempt from the legacy
/// numeric-id identity guard: it is used by offline tests and does not claim
/// to model a real VOICEVOX installation. Character-linked fake fixtures are
/// still resolved normally so the character pipeline remains fully testable.
pub async fn resolve_character_voices(
    config: &mut Config,
    workspace: &Workspace,
    tts: &dyn TtsEngine,
) -> Result<(), AppError> {
    let loaded = load_manifest(config, workspace)?;
    let needs_character_resolution = loaded.is_some();
    let needs_profile_guard = tts.id() != "fake";

    if !needs_character_resolution && !needs_profile_guard {
        return Ok(());
    }

    let engine_speakers = tts.list_speakers().await?;

    if needs_profile_guard {
        let profile_report = crate::speaker_profile::resolve_speaker_profiles_from_list(
            config,
            workspace,
            &engine_speakers,
        )?;
        crate::speaker_profile::ensure_generation_safe_profiles(&profile_report, workspace)?;
    }

    let Some(loaded) = loaded else {
        return Ok(());
    };
    let manifest = &loaded.manifest;
    let keys: Vec<String> = config
        .speakers
        .iter()
        .filter(|(_, s)| s.character_id.is_some())
        .map(|(k, _)| k.clone())
        .collect();

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
        let id = resolve_speaker_style_id(&engine_speakers, &voice.speaker, &voice.style)
            .ok_or_else(|| AppError::VoicevoxSpeakerNotFound {
                speaker: voice.speaker.clone(),
                style: voice.style.clone(),
                known: describe_speakers(&engine_speakers),
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
    async fn fake_plain_speaker_stays_offline_compatible() {
        let (_dir, ws, mut cfg) = workspace_with_config(
            "speakers:\n  reimu:\n    aliases: [霊夢]\n    voice:\n      speaker_id: 2\n",
        );
        let tts = FakeTtsEngine::default();
        resolve_character_voices(&mut cfg, &ws, &tts).await.unwrap();
        assert_eq!(cfg.speakers["reimu"].voice.speaker_id, 2);
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
