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
