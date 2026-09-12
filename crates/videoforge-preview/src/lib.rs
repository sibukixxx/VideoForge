//! FFmpeg preview renderer (design §18–§19).
//!
//! Goal: timing confirmation, not a finished video. Background (image or flat
//! color) + dialogue audio placed at their timeline offsets + captions with
//! the speaker name + a short fade in/out.

pub mod command;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use videoforge_core::preview::{PreviewRenderer, PreviewRequest};
use videoforge_core::AppError;

pub use command::{
    build_args, escape_option_value, quote_filter_value, quote_graph_token, RenderPlan,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInfo {
    pub path: PathBuf,
    pub version: String,
    pub capabilities: FfmpegCapabilities,
}

/// FFmpeg features used by the preview filter graph and default encoder.
/// Keeping this explicit lets `doctor` reject an incomplete FFmpeg build
/// before TTS work starts (most notably builds without `drawtext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegCapabilities {
    pub filters: Vec<String>,
    pub encoders: Vec<String>,
}

impl FfmpegCapabilities {
    const REQUIRED_FILTERS: [&'static str; 10] = [
        "adelay",
        "afade",
        "amix",
        "crop",
        "drawtext",
        "overlay",
        "rotate",
        "scale",
        "volume",
        "dynaudnorm",
    ];
    const REQUIRED_ENCODERS: [&'static str; 2] = ["aac", "libx264"];

    pub fn missing_required(&self) -> Vec<String> {
        let mut missing = Vec::new();
        for required in Self::REQUIRED_FILTERS {
            if !self.filters.iter().any(|value| value == required) {
                missing.push(format!("filter:{required}"));
            }
        }
        for required in Self::REQUIRED_ENCODERS {
            if !self.encoders.iter().any(|value| value == required) {
                missing.push(format!("encoder:{required}"));
            }
        }
        missing
    }
}

/// Locate FFmpeg: `$VIDEOFORGE_FFMPEG`, else `ffmpeg` on `PATH`.
pub fn detect_ffmpeg() -> Result<FfmpegInfo, AppError> {
    let candidate = std::env::var_os("VIDEOFORGE_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let output = std::process::Command::new(&candidate)
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            AppError::FfmpegUnavailable(format!(
                "`{}` could not be executed ({e}); install FFmpeg or set VIDEOFORGE_FFMPEG",
                candidate.display()
            ))
        })?;
    if !output.status.success() {
        return Err(AppError::FfmpegUnavailable(format!(
            "`{} -version` exited with {}",
            candidate.display(),
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.lines().next().unwrap_or("ffmpeg").trim().to_string();
    let capabilities = detect_capabilities(&candidate)?;
    Ok(FfmpegInfo {
        path: candidate,
        version,
        capabilities,
    })
}

fn detect_capabilities(ffmpeg: &Path) -> Result<FfmpegCapabilities, AppError> {
    let filters = command_listing(ffmpeg, "-filters")?;
    let encoders = command_listing(ffmpeg, "-encoders")?;
    Ok(FfmpegCapabilities {
        filters: parse_listing_names(&filters),
        encoders: parse_listing_names(&encoders),
    })
}

fn command_listing(ffmpeg: &Path, argument: &str) -> Result<String, AppError> {
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", argument])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            AppError::FfmpegUnavailable(format!(
                "`{} {argument}` could not be executed ({e})",
                ffmpeg.display()
            ))
        })?;
    if !output.status.success() {
        return Err(AppError::FfmpegUnavailable(format!(
            "`{} {argument}` exited with {}",
            ffmpeg.display(),
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_listing_names(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = fields.next()?;
            let name = fields.next()?;
            if flags.chars().all(|c| c == '.' || c.is_ascii_uppercase()) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct FfmpegPreviewRenderer {
    ffmpeg: Result<FfmpegInfo, String>,
}

impl FfmpegPreviewRenderer {
    /// Detects FFmpeg once; a missing binary is reported via `availability()`.
    pub fn detect() -> Self {
        Self {
            ffmpeg: detect_ffmpeg().map_err(|e| match e {
                AppError::FfmpegUnavailable(reason) => reason,
                other => other.to_string(),
            }),
        }
    }

    pub fn with_binary(path: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: Ok(FfmpegInfo {
                path: path.into(),
                version: "unknown".into(),
                // Explicit injection is used by renderer tests. Detection is
                // intentionally bypassed so a test can supply a wrapper or a
                // binary whose capabilities were already established.
                capabilities: FfmpegCapabilities {
                    filters: FfmpegCapabilities::REQUIRED_FILTERS
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    encoders: FfmpegCapabilities::REQUIRED_ENCODERS
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                },
            }),
        }
    }

    pub fn info(&self) -> Option<&FfmpegInfo> {
        self.ffmpeg.as_ref().ok()
    }
}

impl Default for FfmpegPreviewRenderer {
    fn default() -> Self {
        Self::detect()
    }
}

#[async_trait]
impl PreviewRenderer for FfmpegPreviewRenderer {
    fn id(&self) -> &'static str {
        "ffmpeg"
    }

    fn availability(&self) -> Result<String, AppError> {
        match &self.ffmpeg {
            Ok(info) => {
                let missing = info.capabilities.missing_required();
                if missing.is_empty() {
                    Ok(format!(
                        "{} ({}) — required filters/encoders available",
                        info.version,
                        info.path.display()
                    ))
                } else {
                    Err(AppError::FfmpegUnavailable(format!(
                        "{} is missing required preview capabilities: {}",
                        info.path.display(),
                        missing.join(", ")
                    )))
                }
            }
            Err(e) => Err(AppError::FfmpegUnavailable(e.clone())),
        }
    }

    async fn render(&self, request: PreviewRequest<'_>) -> Result<(), AppError> {
        let info = match &self.ffmpeg {
            Ok(i) => i,
            Err(e) => return Err(AppError::FfmpegUnavailable(e.clone())),
        };
        if request.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }

        let scratch = scratch_dir(request.output);
        std::fs::create_dir_all(&scratch).map_err(|e| AppError::write(&scratch, e))?;
        let plan = RenderPlan::from_request(&request, &scratch)?;
        plan.write_caption_files()?;
        let args = build_args(&plan);

        let result = run_ffmpeg(&info.path, &args, request.project_dir, &request.cancel).await;
        let _ = std::fs::remove_dir_all(&scratch);
        result?;

        if !request.output.is_file() {
            return Err(AppError::PreviewRenderFailed(format!(
                "ffmpeg exited successfully but {} was not written",
                request.output.display()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_filter_and_encoder_listings() {
        let listing = " Filters:\n T.. = Timeline support\n ... drawtext         V->V\n TS. overlay          VV->V\n V..... libx264       H.264 encoder\n A..... aac           AAC encoder\n";
        assert_eq!(
            parse_listing_names(listing),
            ["drawtext", "overlay", "libx264", "aac"]
        );
    }

    #[test]
    fn reports_each_missing_required_capability() {
        let capabilities = FfmpegCapabilities {
            filters: FfmpegCapabilities::REQUIRED_FILTERS
                .iter()
                .filter(|value| **value != "drawtext")
                .map(|value| (*value).to_string())
                .collect(),
            encoders: vec!["aac".into()],
        };
        let missing = capabilities.missing_required();
        assert_eq!(missing, ["filter:drawtext", "encoder:libx264"]);
    }
}

fn scratch_dir(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "preview".into());
    output
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{name}.tmp"))
}

async fn run_ffmpeg(
    ffmpeg: &Path,
    args: &[OsString],
    cwd: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), AppError> {
    let mut child = tokio::process::Command::new(ffmpeg)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AppError::FfmpegUnavailable(format!("failed to start ffmpeg: {e}")))?;

    let mut stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });

    let status = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(AppError::Cancelled);
        }
        s = child.wait() => s.map_err(|e| AppError::PreviewRenderFailed(e.to_string()))?,
    };
    let stderr = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let text = String::from_utf8_lossy(&stderr);
        let tail: Vec<&str> = text
            .lines()
            .rev()
            .take(15)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(AppError::PreviewRenderFailed(format!(
            "ffmpeg exited with {status}:\n{}",
            tail.join("\n")
        )));
    }
    Ok(())
}
