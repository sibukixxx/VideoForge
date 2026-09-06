//! Error payload sent to the frontend.
//!
//! `code` is `AppError::code()` — the same stable strings `videoforge --json`
//! emits — so the UI dispatches on the code, never on the message text.

use serde::Serialize;
use videoforge_core::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// A GUI-level problem that is not an [`AppError`] (bad argument, IPC
    /// misuse). Kept distinct from `AppError::Other` so the code set the CLI
    /// documents is not widened silently.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }
}

impl From<AppError> for CommandError {
    fn from(e: AppError) -> Self {
        Self {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

impl From<videoforge_core::project::ProjectError> for CommandError {
    fn from(e: videoforge_core::project::ProjectError) -> Self {
        Self::from(AppError::from(e))
    }
}

impl From<videoforge_platform::PlatformError> for CommandError {
    fn from(e: videoforge_platform::PlatformError) -> Self {
        Self::from(AppError::from(e))
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

pub type CommandResult<T> = Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_error_code_is_forwarded_verbatim() {
        let err = CommandError::from(AppError::FfmpegUnavailable("nope".into()));
        assert_eq!(err.code, "ffmpeg_unavailable");
        assert!(err.message.contains("nope"));
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "ffmpeg_unavailable");
    }
}
