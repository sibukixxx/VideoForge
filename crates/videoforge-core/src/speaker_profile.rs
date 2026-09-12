//! Canonical speaker-profile resolution for issue #46.
//!
//! A speaker profile is the one place where the script-facing identity
//! (canonical key + aliases), configured VOICEVOX style, optional character,
//! visual assets, subtitle color, and voice scales are tied together.
//! CLI/Tauri/preflight code should consume this module rather than attempting
//! to re-resolve those pieces independently.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::character::{load_manifest, resolve_png_sprites};
use crate::config::{Config, VoiceParams};
use crate::error::AppError;
use crate::tts::{Speaker, TtsEngine};
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedVoiceIdentity {
    pub speaker: String,
    pub style: String,
    pub style_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCharacterAssets {
    pub character_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerProfile {
    pub key: String,
    pub aliases: Vec<String>,
    pub display_name: String,
    pub voice: VoiceParams,
    pub resolved_voice: ResolvedVoiceIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<ResolvedCharacterAssets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_color: Option<String>,
    /// `true` for the legacy numeric-id-only form. This is intentionally
    /// surfaced so doctor / GUI can warn that the script-visible identity is
    /// not pinned to a named VOICEVOX speaker/style yet.
    pub legacy_unpinned_voice: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerProfileWarning {
    pub speaker_key: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SpeakerProfileReport {
    pub profiles: Vec<SpeakerProfile>,
    pub warnings: Vec<SpeakerProfileWarning>,
}

/// Resolve every configured speaker against one VOICEVOX speaker/style list.
///
/// Character-linked speakers are authoritative by *name* from the character
/// manifest. Plain speakers retain backward compatibility with the historical
/// numeric `speaker_id`, but are marked `legacy_unpinned_voice` and reported
/// with the actual VOICEVOX speaker/style that the id means. This is the
/// migration bridge for old `reimu`/`marisa` workspaces: they are no longer
/// opaque numeric ids during preflight, without hard-coding any character
/// names in core.
pub async fn resolve_speaker_profiles(
    config: &Config,
    workspace: &Workspace,
    tts: &dyn TtsEngine,
) -> Result<SpeakerProfileReport, AppError> {
    let engine_speakers = tts.list_speakers().await?;
    resolve_speaker_profiles_from_list(config, workspace, &engine_speakers)
}

pub fn resolve_speaker_profiles_from_list(
    config: &Config,
    workspace: &Workspace,
    engine_speakers: &[Speaker],
) -> Result<SpeakerProfileReport, AppError> {
    let loaded_manifest = load_manifest(config, workspace)?;
    let mut report = SpeakerProfileReport::default();

    for (key, speaker_cfg) in &config.speakers {
        let display_name = speaker_cfg
            .aliases
            .first()
            .cloned()
            .unwrap_or_else(|| key.clone());

        let (resolved_voice, character, legacy_unpinned_voice) = if let Some(character_id) =
            &speaker_cfg.character_id
        {
            let loaded = loaded_manifest.as_ref().ok_or_else(|| AppError::InvalidConfig {
                    path: workspace.root().join("videoforge.yaml"),
                    reason: format!(
                        "speakers.{key}.character_id is set but character_manifest could not be loaded"
                    ),
                })?;
            let character =
                loaded
                    .manifest
                    .find(character_id)
                    .ok_or_else(|| AppError::CharacterNotFound {
                        id: character_id.clone(),
                        known: loaded.manifest.character_ids().join(", "),
                    })?;

            let identity = if let Some(voice) = &character.voice {
                find_named_voice(engine_speakers, &voice.speaker, &voice.style).ok_or_else(
                    || AppError::VoicevoxSpeakerNotFound {
                        speaker: voice.speaker.clone(),
                        style: voice.style.clone(),
                        known: describe_speakers(engine_speakers),
                    },
                )?
            } else {
                find_style_id(engine_speakers, speaker_cfg.voice.speaker_id).ok_or_else(|| {
                    invalid_style_id(
                        workspace,
                        key,
                        speaker_cfg.voice.speaker_id,
                        engine_speakers,
                    )
                })?
            };

            let manifest_dir = loaded.path.parent().unwrap_or_else(|| Path::new("."));
            let model_type = character.model.as_ref().map(|m| m.model_type.clone());
            let sprites = resolve_png_sprites(character, manifest_dir)?;
            let assets = ResolvedCharacterAssets {
                character_id: character_id.clone(),
                display_name: character.display_name.clone(),
                model_type,
                closed: sprites.as_ref().map(|s| s.closed.display().to_string()),
                half: sprites.as_ref().map(|s| s.half.display().to_string()),
                open: sprites.as_ref().map(|s| s.open.display().to_string()),
            };
            (identity, Some(assets), false)
        } else {
            let identity = find_style_id(engine_speakers, speaker_cfg.voice.speaker_id)
                .ok_or_else(|| {
                    invalid_style_id(
                        workspace,
                        key,
                        speaker_cfg.voice.speaker_id,
                        engine_speakers,
                    )
                })?;
            report.warnings.push(SpeakerProfileWarning {
                    speaker_key: key.clone(),
                    message: format!(
                        "speaker `{key}` uses legacy numeric-only VOICEVOX style id {}; it currently resolves to `{}` / `{}`. Pin identity with character_id + character_manifest before relying on the script-visible name `{display_name}`",
                        identity.style_id, identity.speaker, identity.style
                    ),
                });
            (identity, None, true)
        };

        report.profiles.push(SpeakerProfile {
            key: key.clone(),
            aliases: speaker_cfg.aliases.clone(),
            display_name,
            voice: speaker_cfg.voice,
            resolved_voice,
            character,
            caption_color: speaker_cfg.caption_color.clone(),
            legacy_unpinned_voice,
        });
    }

    Ok(report)
}

fn invalid_style_id(
    workspace: &Workspace,
    key: &str,
    style_id: u32,
    engine_speakers: &[Speaker],
) -> AppError {
    AppError::InvalidConfig {
        path: workspace.root().join("videoforge.yaml"),
        reason: format!(
            "speakers.{key}.voice.speaker_id={style_id} does not exist in the connected VOICEVOX engine (known: {})",
            describe_speakers(engine_speakers)
        ),
    }
}

fn find_style_id(speakers: &[Speaker], style_id: u32) -> Option<ResolvedVoiceIdentity> {
    speakers.iter().find_map(|speaker| {
        speaker
            .styles
            .iter()
            .find(|style| style.id == style_id)
            .map(|style| ResolvedVoiceIdentity {
                speaker: speaker.name.clone(),
                style: style.name.clone(),
                style_id: style.id,
            })
    })
}

fn find_named_voice(
    speakers: &[Speaker],
    name: &str,
    style: &str,
) -> Option<ResolvedVoiceIdentity> {
    speakers
        .iter()
        .find(|speaker| speaker.name == name)
        .and_then(|speaker| {
            speaker
                .styles
                .iter()
                .find(|candidate| candidate.name == style)
                .map(|candidate| ResolvedVoiceIdentity {
                    speaker: speaker.name.clone(),
                    style: candidate.name.clone(),
                    style_id: candidate.id,
                })
        })
}

fn describe_speakers(speakers: &[Speaker]) -> String {
    speakers
        .iter()
        .map(|speaker| {
            let styles = speaker
                .styles
                .iter()
                .map(|style| format!("{}:{}", style.name, style.id))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} ({styles})", speaker.name)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::init;
    use crate::tts::SpeakerStyle;

    fn workspace_with_config(yaml: &str) -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("profiles")).unwrap();
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(yaml, dir.path()).unwrap();
        (dir, ws, cfg)
    }

    fn engine() -> Vec<Speaker> {
        vec![
            Speaker {
                name: "四国めたん".into(),
                styles: vec![SpeakerStyle {
                    id: 2,
                    name: "ノーマル".into(),
                }],
            },
            Speaker {
                name: "ずんだもん".into(),
                styles: vec![SpeakerStyle {
                    id: 3,
                    name: "ノーマル".into(),
                }],
            },
        ]
    }

    #[test]
    fn legacy_numeric_speaker_is_resolved_but_explicitly_warned() {
        let (_dir, ws, cfg) = workspace_with_config(
            "speakers:\n  reimu:\n    aliases: [霊夢]\n    voice:\n      speaker_id: 2\n",
        );
        let report = resolve_speaker_profiles_from_list(&cfg, &ws, &engine()).unwrap();
        assert_eq!(report.profiles.len(), 1);
        let p = &report.profiles[0];
        assert_eq!(p.key, "reimu");
        assert_eq!(p.display_name, "霊夢");
        assert_eq!(p.resolved_voice.speaker, "四国めたん");
        assert_eq!(p.resolved_voice.style, "ノーマル");
        assert!(p.legacy_unpinned_voice);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].message.contains("霊夢"));
        assert!(report.warnings[0].message.contains("四国めたん"));
    }

    #[test]
    fn missing_numeric_style_is_a_typed_invalid_config() {
        let (_dir, ws, cfg) =
            workspace_with_config("speakers:\n  narrator:\n    voice:\n      speaker_id: 9999\n");
        let err = resolve_speaker_profiles_from_list(&cfg, &ws, &engine()).unwrap_err();
        assert_eq!(err.code(), "invalid_config");
        assert!(err
            .to_string()
            .contains("speakers.narrator.voice.speaker_id"));
        assert!(err.to_string().contains("9999"));
    }

    #[test]
    fn profile_preserves_voice_scales_and_caption_color() {
        let (_dir, ws, cfg) = workspace_with_config(
            "speakers:\n  zundamon:\n    aliases: [ずんだもん]\n    caption_color: '#88ff88'\n    voice:\n      speaker_id: 3\n      speed_scale: 1.2\n      pitch_scale: 0.01\n      intonation_scale: 1.1\n      volume_scale: 0.9\n",
        );
        let report = resolve_speaker_profiles_from_list(&cfg, &ws, &engine()).unwrap();
        let p = &report.profiles[0];
        assert_eq!(p.caption_color.as_deref(), Some("#88ff88"));
        assert_eq!(
            p.voice,
            VoiceParams {
                speaker_id: 3,
                speed_scale: 1.2,
                pitch_scale: 0.01,
                intonation_scale: 1.1,
                volume_scale: 0.9,
            }
        );
    }
}
