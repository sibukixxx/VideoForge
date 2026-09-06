//! Exporter abstraction (design §52).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use videoforge_project::VideoProject;

use crate::error::AppError;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportCapabilities {
    /// Can this exporter materialize output on the current platform?
    pub available: bool,
    /// Why not (shown to the user, not treated as a failure).
    pub reason: Option<String>,
    /// Can the produced file be opened in its native editor here?
    pub can_open: bool,
}

pub struct ExportRequest<'a> {
    pub project: &'a VideoProject,
    pub project_path: &'a Path,
    /// Directory the project's relative asset paths resolve against.
    pub project_dir: &'a Path,
    pub workspace: Option<&'a Workspace>,
    /// Explicit template override.
    pub template: Option<&'a Path>,
    /// Explicit output path override.
    pub destination: Option<&'a Path>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    pub output: PathBuf,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[async_trait]
pub trait ProjectExporter: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> ExportCapabilities;
    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, AppError>;
}
