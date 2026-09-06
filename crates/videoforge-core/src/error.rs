use std::path::PathBuf;

use thiserror::Error;

/// Error model shared by CLI and GUI (design §29).
#[derive(Debug, Error)]
pub enum AppError {
    #[error("workspace not found: no videoforge.yaml in {0} or any parent directory")]
    WorkspaceNotFound(PathBuf),
    #[error("invalid config {path}: {reason}")]
    InvalidConfig { path: PathBuf, reason: String },
    #[error("invalid script: {0}")]
    InvalidScript(String),
    #[error("unknown speaker `{speaker}` on line {line} (known speakers: {known})")]
    UnknownSpeaker {
        speaker: String,
        line: usize,
        known: String,
    },

    #[error("VOICEVOX is unavailable at {endpoint}: {reason}")]
    VoicevoxUnavailable { endpoint: String, reason: String },
    #[error("VOICEVOX synthesis failed for dialogue {index} ({speaker}): {reason}")]
    VoicevoxSynthesisFailed {
        index: usize,
        speaker: String,
        reason: String,
    },

    #[error("FFmpeg is unavailable: {0}")]
    FfmpegUnavailable(String),
    #[error("preview render failed: {0}")]
    PreviewRenderFailed(String),

    #[error("invalid template {path}: {reason}")]
    InvalidTemplate { path: PathBuf, reason: String },
    #[error("template prototype missing: {0}")]
    TemplatePrototypeMissing(String),

    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),
    #[error("YukkuriMovieMaker4 is unavailable: {0}")]
    Ymm4Unavailable(String),

    #[error("failed to read {path}: {source}")]
    FileReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    FileWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("operation cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Stable machine-readable code (for JSON output / GUI dispatch).
    pub fn code(&self) -> &'static str {
        match self {
            AppError::WorkspaceNotFound(_) => "workspace_not_found",
            AppError::InvalidConfig { .. } => "invalid_config",
            AppError::InvalidScript(_) => "invalid_script",
            AppError::UnknownSpeaker { .. } => "unknown_speaker",
            AppError::VoicevoxUnavailable { .. } => "voicevox_unavailable",
            AppError::VoicevoxSynthesisFailed { .. } => "voicevox_synthesis_failed",
            AppError::FfmpegUnavailable(_) => "ffmpeg_unavailable",
            AppError::PreviewRenderFailed(_) => "preview_render_failed",
            AppError::InvalidTemplate { .. } => "invalid_template",
            AppError::TemplatePrototypeMissing(_) => "template_prototype_missing",
            AppError::UnsupportedPlatform(_) => "unsupported_platform",
            AppError::Ymm4Unavailable(_) => "ymm4_unavailable",
            AppError::FileReadFailed { .. } => "file_read_failed",
            AppError::FileWriteFailed { .. } => "file_write_failed",
            AppError::Cancelled => "cancelled",
            AppError::Other(_) => "other",
        }
    }

    pub fn read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AppError::FileReadFailed {
            path: path.into(),
            source,
        }
    }

    pub fn write(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AppError::FileWriteFailed {
            path: path.into(),
            source,
        }
    }
}

impl From<videoforge_script::ScriptError> for AppError {
    fn from(e: videoforge_script::ScriptError) -> Self {
        match e {
            videoforge_script::ScriptError::Read { path, source } => AppError::read(path, source),
            other => AppError::InvalidScript(other.to_string()),
        }
    }
}

impl From<videoforge_project::ProjectError> for AppError {
    fn from(e: videoforge_project::ProjectError) -> Self {
        use videoforge_project::ProjectError as P;
        match e {
            P::Read { path, source } => AppError::read(path, source),
            P::Write { path, source } => AppError::write(path, source),
            other => AppError::Other(other.to_string()),
        }
    }
}

impl From<videoforge_project::PathError> for AppError {
    fn from(e: videoforge_project::PathError) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<videoforge_timeline::TimelineError> for AppError {
    fn from(e: videoforge_timeline::TimelineError) -> Self {
        AppError::Other(format!("timeline: {e}"))
    }
}

impl From<videoforge_platform::PlatformError> for AppError {
    fn from(e: videoforge_platform::PlatformError) -> Self {
        AppError::UnsupportedPlatform(e.to_string())
    }
}
