//! Links a project's speakers to entries in a standalone character manifest
//! (design §5) and resolves VOICEVOX voices by *name* instead of a
//! hard-coded numeric id.
//!
//! This module is the only place that turns `SpeakerConfig::character_id`
//! into anything: everything downstream (TTS synthesis, the cache key,
//! `videoforge-timeline`) keeps working with the plain numeric
//! `VoiceParams::speaker_id` it always has, so a workspace that never sets
//! `character_id` is completely unaffected by this feature existing.

use std::path::PathBuf;

use videoforge_character::CharacterManifest;

use crate::config::Config;
use crate::error::AppError;
use crate::tts::{Speaker, TtsEngine};
use crate::workspace::Workspace;

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
}
