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

pub use command::{build_args, RenderPlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegInfo {
    pub path: PathBuf,
    pub version: String,
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
    Ok(FfmpegInfo {
        path: candidate,
        version,
    })
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
            Ok(info) => Ok(format!("{} ({})", info.version, info.path.display())),
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
