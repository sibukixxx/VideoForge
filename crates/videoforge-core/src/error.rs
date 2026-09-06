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
    #[error("invalid workspace path `{path}`: {reason}")]
    InvalidWorkspacePath { path: String, reason: String },
    #[error("remote TTS endpoint `{endpoint}` is not allowed (only localhost/127.0.0.1/::1 are permitted; set tts.allow_remote_endpoint: true to opt in)")]
    RemoteEndpointNotAllowed { endpoint: String },
    #[error("refusing to overwrite {path}: {reason}")]
    UnsafeOverwrite { path: PathBuf, reason: String },
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
    #[error(
        "template {path} has {} candidate prototype containers ({}); \
         keep prototype items (Remark starting with `VF_PROTO_`) in exactly one timeline",
        candidates.len(),
        candidates.join(", ")
    )]
    TemplatePrototypeAmbiguous {
        path: PathBuf,
        /// JSON pointers of every array holding prototype items.
        candidates: Vec<String>,
    },

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

    #[error("invalid asset path: {source}")]
    InvalidAssetPath {
        #[source]
        source: videoforge_project::PathError,
    },
    #[error("invalid project data: {source}")]
    ProjectInvalid {
        #[source]
        source: videoforge_project::ProjectError,
    },
    #[error("timeline build failed: {source}")]
    TimelineBuildFailed {
        #[source]
        source: videoforge_timeline::TimelineError,
    },
    #[error("failed to serialize {what}: {source}")]
    SerializationFailed {
        what: String,
        #[source]
        source: serde_json::Error,
    },
    /// A bug in VideoForge, not a user or environment problem.
    #[error("internal invariant violated: {0}")]
    InternalInvariant(String),

    #[error("another generate is already running for `{slug}` (lock: {lock_path})")]
    Busy { slug: String, lock_path: PathBuf },

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
            AppError::InvalidWorkspacePath { .. } => "invalid_workspace_path",
            AppError::RemoteEndpointNotAllowed { .. } => "remote_endpoint_not_allowed",
            AppError::UnsafeOverwrite { .. } => "unsafe_overwrite",
            AppError::UnknownSpeaker { .. } => "unknown_speaker",
            AppError::VoicevoxUnavailable { .. } => "voicevox_unavailable",
            AppError::VoicevoxSynthesisFailed { .. } => "voicevox_synthesis_failed",
            AppError::FfmpegUnavailable(_) => "ffmpeg_unavailable",
            AppError::PreviewRenderFailed(_) => "preview_render_failed",
            AppError::InvalidTemplate { .. } => "invalid_template",
            AppError::TemplatePrototypeMissing(_) => "template_prototype_missing",
            AppError::TemplatePrototypeAmbiguous { .. } => "template_prototype_ambiguous",
            AppError::UnsupportedPlatform(_) => "unsupported_platform",
            AppError::Ymm4Unavailable(_) => "ymm4_unavailable",
            AppError::FileReadFailed { .. } => "file_read_failed",
            AppError::FileWriteFailed { .. } => "file_write_failed",
            AppError::InvalidAssetPath { .. } => "invalid_asset_path",
            AppError::ProjectInvalid { .. } => "project_invalid",
            AppError::TimelineBuildFailed { .. } => "timeline_build_failed",
            AppError::SerializationFailed { .. } => "serialization_failed",
            AppError::InternalInvariant(_) => "internal_invariant",
            AppError::Busy { .. } => "busy",
            AppError::Cancelled => "cancelled",
            AppError::Other(_) => "other",
        }
    }

    /// A `serde_json` failure while writing `what` (a file name or a short
    /// description of the value).
    pub fn serialization(what: impl Into<String>, source: serde_json::Error) -> Self {
        AppError::SerializationFailed {
            what: what.into(),
            source,
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
            other => AppError::ProjectInvalid { source: other },
        }
    }
}

impl From<videoforge_project::PathError> for AppError {
    fn from(e: videoforge_project::PathError) -> Self {
        AppError::InvalidAssetPath { source: e }
    }
}

impl From<videoforge_timeline::TimelineError> for AppError {
    fn from(e: videoforge_timeline::TimelineError) -> Self {
        AppError::TimelineBuildFailed { source: e }
    }
}

impl From<videoforge_platform::PlatformError> for AppError {
    fn from(e: videoforge_platform::PlatformError) -> Self {
        AppError::UnsupportedPlatform(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn path_errors_get_a_dedicated_code_and_keep_their_source() {
        let path_err = videoforge_project::RelativeAssetPath::new("../escape.wav").unwrap_err();
        let err = AppError::from(path_err);

        assert_eq!(err.code(), "invalid_asset_path");
        assert!(err.to_string().contains("../escape.wav"));
        assert!(err.source().is_some(), "PathError must stay reachable");
    }

    #[test]
    fn timeline_errors_get_a_dedicated_code_and_keep_their_source() {
        let err = AppError::from(videoforge_timeline::TimelineError::Empty);

        assert_eq!(err.code(), "timeline_build_failed");
        assert!(err.source().is_some());
    }

    #[test]
    fn project_schema_errors_get_a_dedicated_code() {
        let err = AppError::from(videoforge_project::ProjectError::UnsupportedSchema {
            found: 99,
            supported: 1,
        });

        assert_eq!(err.code(), "project_invalid");
        assert!(err.to_string().contains("99"));
        assert!(err.source().is_some());
    }

    #[test]
    fn project_io_errors_still_map_to_read_and_write() {
        let read = AppError::from(videoforge_project::ProjectError::Read {
            path: "p.json".into(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        let write = AppError::from(videoforge_project::ProjectError::Write {
            path: "p.json".into(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        });

        assert_eq!(read.code(), "file_read_failed");
        assert_eq!(write.code(), "file_write_failed");
    }

    #[test]
    fn serialization_failures_keep_the_serde_error_as_source() {
        let json_err = serde_json::from_str::<serde_json::Value>("{oops").unwrap_err();
        let err = AppError::serialization("manifest.json", json_err);

        assert_eq!(err.code(), "serialization_failed");
        assert!(err.to_string().contains("manifest.json"));
        assert!(err.source().is_some());
    }

    #[test]
    fn codes_are_unique_per_variant() {
        let codes = [
            AppError::WorkspaceNotFound("x".into()).code(),
            AppError::InvalidAssetPath {
                source: videoforge_project::PathError::Empty,
            }
            .code(),
            AppError::TimelineBuildFailed {
                source: videoforge_timeline::TimelineError::Empty,
            }
            .code(),
            AppError::InternalInvariant("x".into()).code(),
            AppError::Busy {
                slug: "s".into(),
                lock_path: "l".into(),
            }
            .code(),
            AppError::TemplatePrototypeAmbiguous {
                path: "t.ymmp".into(),
                candidates: vec!["/a".into(), "/b".into()],
            }
            .code(),
            AppError::Other("x".into()).code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            codes.len(),
            "duplicate error codes: {codes:?}"
        );
    }
}
