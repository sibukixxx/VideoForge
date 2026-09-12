//! `videoforge doctor` (design §7.2). Missing YMM4 on macOS is a capability,
//! not a failure.

use serde::{Deserialize, Serialize};
use videoforge_platform::{platform_label, Platform, PlatformKind};

use crate::capabilities::Capabilities;
use crate::error::AppError;
use crate::export::ProjectExporter;
use crate::preview::PreviewRenderer;
use crate::tts::TtsEngine;
use crate::workspace::Workspace;

/// Below this, `doctor` warns before a generate is attempted (P0-3). Not a
/// hard engine limit — a short synthetic video's WAVs + preview.mp4 rarely
/// exceed a few tens of MB, but leaves headroom for a longer real one.
const MIN_FREE_DISK_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    /// Not an error: the feature does not exist on this platform.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
    pub capabilities: Capabilities,
}

impl DoctorReport {
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }
}

pub struct DoctorInput<'a> {
    pub workspace: Result<Workspace, AppError>,
    pub tts: &'a dyn TtsEngine,
    pub tts_endpoint: String,
    pub preview: Option<&'a dyn PreviewRenderer>,
    pub exporter: Option<&'a dyn ProjectExporter>,
    pub platform: &'a dyn Platform,
}

pub async fn run(input: DoctorInput<'_>) -> DoctorReport {
    let mut checks = Vec::new();
    let push = |checks: &mut Vec<DoctorCheck>, name: &str, status: CheckStatus, detail: String| {
        checks.push(DoctorCheck {
            name: name.into(),
            status,
            detail,
        })
    };

    // Workspace + config
    let mut config = None;
    let workspace = match &input.workspace {
        Ok(ws) => {
            push(
                &mut checks,
                "Workspace",
                CheckStatus::Ok,
                ws.root().display().to_string(),
            );
            match ws.load_config() {
                Ok(cfg) => {
                    push(
                        &mut checks,
                        "Config",
                        CheckStatus::Ok,
                        format!(
                            "{} speaker(s): {}",
                            cfg.speakers.len(),
                            cfg.known_speaker_names().join(", ")
                        ),
                    );
                    config = Some(cfg);
                }
                Err(e) => push(&mut checks, "Config", CheckStatus::Fail, e.to_string()),
            }
            Some(ws)
        }
        Err(e) => {
            push(&mut checks, "Workspace", CheckStatus::Fail, e.to_string());
            None
        }
    };

    // VOICEVOX
    let voicevox_available = match input.tts.health().await {
        Ok(info) => {
            push(
                &mut checks,
                "VOICEVOX connection",
                CheckStatus::Ok,
                format!("{} {} at {}", info.engine, info.version, info.endpoint),
            );
            true
        }
        Err(e) => {
            push(
                &mut checks,
                "VOICEVOX connection",
                CheckStatus::Fail,
                e.to_string(),
            );
            false
        }
    };

    // FFmpeg
    let ffmpeg_available = match input.preview.map(|p| p.availability()) {
        Some(Ok(desc)) => {
            push(&mut checks, "FFmpeg", CheckStatus::Ok, desc);
            true
        }
        Some(Err(e)) => {
            push(
                &mut checks,
                "FFmpeg",
                CheckStatus::Warn,
                format!("{e} (preview.mp4 will be skipped)"),
            );
            false
        }
        None => {
            push(
                &mut checks,
                "FFmpeg",
                CheckStatus::Warn,
                "no preview renderer configured".into(),
            );
            false
        }
    };

    // Characters (P0-3): identity → voice → visual asset must all resolve
    // *before* a generate is attempted. Only speakers that actually link a
    // character are checked — a workspace that doesn't use the feature at
    // all gets no new check, matching every other additive character
    // behavior in this codebase.
    if let (Some(ws), Some(cfg)) = (workspace, &config) {
        match crate::character::load_manifest(cfg, ws) {
            Ok(Some(loaded)) => {
                let manifest_dir = loaded
                    .path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                let mut speakers_cache: Option<Vec<crate::tts::Speaker>> = None;
                let mut character_ids: Vec<&str> = cfg
                    .speakers
                    .values()
                    .filter_map(|s| s.character_id.as_deref())
                    .collect();
                character_ids.sort_unstable();
                character_ids.dedup();
                for character_id in character_ids {
                    let Some(character) = loaded.manifest.find(character_id) else {
                        push(
                            &mut checks,
                            &format!("Character `{character_id}`"),
                            CheckStatus::Fail,
                            format!(
                                "not found in {} (known: {})",
                                loaded.path.display(),
                                loaded.manifest.character_ids().join(", ")
                            ),
                        );
                        continue;
                    };
                    let mut problems = Vec::new();
                    if let Some(voice) = &character.voice {
                        if speakers_cache.is_none() && voicevox_available {
                            speakers_cache = input.tts.list_speakers().await.ok();
                        }
                        match &speakers_cache {
                            Some(speakers) => {
                                let found =
                                    speakers.iter().find(|s| s.name == voice.speaker).and_then(
                                        |s| s.styles.iter().find(|st| st.name == voice.style),
                                    );
                                if found.is_none() {
                                    // Never silently fall back to a different speaker
                                    // (design requirement: Reimu/Marisa must not
                                    // resolve to an arbitrary VOICEVOX voice).
                                    problems.push(format!(
                                        "VOICEVOX has no speaker `{}` with style `{}`",
                                        voice.speaker, voice.style
                                    ));
                                }
                            }
                            None => problems
                                .push("cannot verify voice: VOICEVOX is unavailable".to_string()),
                        }
                    }
                    match &character.model {
                        Some(model) if model.is_live2d() => {
                            let resolved = model.resolve_path(manifest_dir);
                            if let Err(e) =
                                videoforge_character::live2d::load_model3_json(&resolved)
                            {
                                problems.push(e.to_string());
                            }
                        }
                        Some(model) if model.is_png_lipsync() => {
                            if let Err(e) =
                                videoforge_character::png_lipsync::load_png_lipsync_assets(
                                    model,
                                    manifest_dir,
                                )
                            {
                                problems.push(e.to_string());
                            }
                        }
                        _ => {}
                    }
                    let status = if problems.is_empty() {
                        CheckStatus::Ok
                    } else {
                        CheckStatus::Fail
                    };
                    let detail = if problems.is_empty() {
                        format!("{} OK", character.display_name)
                    } else {
                        format!("{}: {}", character.display_name, problems.join("; "))
                    };
                    push(
                        &mut checks,
                        &format!("Character `{character_id}`"),
                        status,
                        detail,
                    );
                }
            }
            Ok(None) => {}
            Err(e) => push(&mut checks, "Characters", CheckStatus::Fail, e.to_string()),
        }
    }

    // Output directory
    if let Some(ws) = workspace {
        let out = ws.generated_dir();
        let status = match std::fs::create_dir_all(&out) {
            Ok(()) => {
                let probe = out.join(".vf-doctor-write-test");
                match std::fs::write(&probe, b"ok") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&probe);
                        (CheckStatus::Ok, out.display().to_string())
                    }
                    Err(e) => (
                        CheckStatus::Fail,
                        format!("{} not writable: {e}", out.display()),
                    ),
                }
            }
            Err(e) => (
                CheckStatus::Fail,
                format!("cannot create {}: {e}", out.display()),
            ),
        };
        push(&mut checks, "Output directory", status.0, status.1);

        // Disk space (P0-3): a generate that runs out of space mid-render
        // leaves a stray `.generated-tmp` directory and a confusing FFmpeg
        // error; this catches the common case up front instead.
        match fs4::available_space(&out) {
            Ok(available) => {
                let mb = available / (1024 * 1024);
                if available < MIN_FREE_DISK_BYTES {
                    push(
                        &mut checks,
                        "Disk space",
                        CheckStatus::Warn,
                        format!(
                            "only {mb} MiB free at {} (recommend at least {} MiB)",
                            out.display(),
                            MIN_FREE_DISK_BYTES / (1024 * 1024)
                        ),
                    );
                } else {
                    push(
                        &mut checks,
                        "Disk space",
                        CheckStatus::Ok,
                        format!("{mb} MiB free at {}", out.display()),
                    );
                }
            }
            Err(e) => push(
                &mut checks,
                "Disk space",
                CheckStatus::Warn,
                format!("could not determine free space at {}: {e}", out.display()),
            ),
        }

        // Output settings (P0-3): surfaces resolution/fps up front — actual
        // per-script duration is unknown here (`doctor` takes no script),
        // but a garbled `videoforge.yaml` value is worth flagging before a
        // long TTS run rather than after.
        if let Some(cfg) = &config {
            push(
                &mut checks,
                "Output settings",
                CheckStatus::Ok,
                format!(
                    "{}x{} @ {}fps",
                    cfg.video.width, cfg.video.height, cfg.video.fps
                ),
            );
        }

        // Template
        if let Some(cfg) = &config {
            match ws.resolve(&cfg.export.ymm4.template) {
                Ok(template) if template.is_file() => {
                    push(
                        &mut checks,
                        "Template",
                        CheckStatus::Ok,
                        cfg.export.ymm4.template.clone(),
                    );
                }
                Ok(_) => {
                    push(
                        &mut checks,
                        "Template",
                        CheckStatus::Warn,
                        format!(
                            "{} not found (needed only for `export ymm4`)",
                            cfg.export.ymm4.template
                        ),
                    );
                }
                Err(e) => {
                    push(
                        &mut checks,
                        "Template",
                        CheckStatus::Fail,
                        format!(
                            "invalid export.ymm4.template `{}`: {e}",
                            cfg.export.ymm4.template
                        ),
                    );
                }
            }
        }
    }

    // Platform + YMM4 capability
    let kind = input.platform.kind();
    let ymm4_path = input.platform.find_ymm4();
    let (can_export_ymm4, export_reason) = match input.exporter {
        Some(exp) => {
            let caps = exp.capabilities();
            (caps.available, caps.reason)
        }
        None => (false, Some("no YMM4 exporter configured".into())),
    };
    let can_open_ymm4 = ymm4_path.is_some();
    push(&mut checks, "Platform", CheckStatus::Ok, platform_label());
    match (kind, can_export_ymm4, &ymm4_path) {
        (_, true, Some(p)) => push(&mut checks, "YMM4", CheckStatus::Ok, p.display().to_string()),
        (PlatformKind::Windows, true, None) => push(
            &mut checks,
            "YMM4",
            CheckStatus::Warn,
            "export works, but YukkuriMovieMaker.exe was not found (set VIDEOFORGE_YMM4_PATH to enable `--open`)".into(),
        ),
        _ => push(
            &mut checks,
            "YMM4",
            CheckStatus::Unavailable,
            export_reason.unwrap_or_else(|| format!("unavailable on {}", kind.display_name())),
        ),
    }

    DoctorReport {
        checks,
        capabilities: Capabilities {
            platform: kind,
            platform_label: platform_label(),
            voicevox_available,
            voicevox_endpoint: input.tts_endpoint,
            ffmpeg_available,
            can_export_ymm4,
            can_open_ymm4,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use crate::tts::FakeTtsEngine;
    use videoforge_platform::current_platform;

    fn png_character_workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/character/mock-png-character"
        ));
        let dest_sprites = dir.path().join("characters/sprites");
        std::fs::create_dir_all(&dest_sprites).unwrap();
        std::fs::copy(
            fixture.join("manifest.yaml"),
            dir.path().join("characters/manifest.yaml"),
        )
        .unwrap();
        for name in ["closed.png", "half.png", "open.png"] {
            std::fs::copy(fixture.join("sprites").join(name), dest_sprites.join(name)).unwrap();
        }
        let yaml = "character_manifest: characters/manifest.yaml\nspeakers:\n  mock_a:\n    character_id: mock_a\n    voice:\n      speaker_id: 0\n";
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        (dir, ws)
    }

    async fn run_doctor(ws: Workspace) -> DoctorReport {
        let tts = FakeTtsEngine::default();
        let platform = current_platform();
        run(DoctorInput {
            workspace: Ok(ws),
            tts: &tts,
            tts_endpoint: "http://127.0.0.1:50021".into(),
            preview: None,
            exporter: None,
            platform: platform.as_ref(),
        })
        .await
    }

    #[tokio::test]
    async fn workspace_without_a_character_manifest_has_no_character_checks() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let report = run_doctor(ws).await;
        assert!(!report
            .checks
            .iter()
            .any(|c| c.name.starts_with("Character")));
    }

    #[tokio::test]
    async fn reports_disk_space_and_output_settings() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let report = run_doctor(ws).await;

        let disk = report
            .checks
            .iter()
            .find(|c| c.name == "Disk space")
            .expect("a Disk space check");
        assert_ne!(disk.status, CheckStatus::Fail, "{}", disk.detail);

        let output = report
            .checks
            .iter()
            .find(|c| c.name == "Output settings")
            .expect("an Output settings check");
        assert_eq!(output.status, CheckStatus::Ok);
        assert_eq!(output.detail, "1920x1080 @ 30fps");
    }

    #[tokio::test]
    async fn linked_character_with_valid_sprites_and_a_resolvable_voice_passes() {
        let (_dir, ws) = png_character_workspace();
        let report = run_doctor(ws).await;
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "Character `mock_a`")
            .expect("a Character check for mock_a");
        assert_eq!(check.status, CheckStatus::Ok, "{}", check.detail);
    }

    #[tokio::test]
    async fn linked_character_with_a_missing_sprite_fails_before_generate() {
        let (dir, ws) = png_character_workspace();
        std::fs::remove_file(dir.path().join("characters/sprites/open.png")).unwrap();
        let report = run_doctor(ws).await;
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "Character `mock_a`")
            .unwrap();
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(report.has_failures());
    }

    #[tokio::test]
    async fn unknown_character_id_fails_with_a_clear_message() {
        let (dir, _ws) = png_character_workspace();
        let yaml = "character_manifest: characters/manifest.yaml\nspeakers:\n  mock_a:\n    character_id: ghost\n";
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let report = run_doctor(ws).await;
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "Character `ghost`")
            .unwrap();
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("not found"));
    }
}
