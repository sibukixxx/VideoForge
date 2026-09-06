//! Preview renderer abstraction (design §19).

use std::path::Path;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use videoforge_project::VideoProject;

use crate::error::AppError;

pub struct PreviewRequest<'a> {
    pub project: &'a VideoProject,
    /// Directory the project's relative asset paths resolve against.
    pub project_dir: &'a Path,
    pub output: &'a Path,
    /// Optional font file for captions.
    pub font: Option<&'a Path>,
    /// Flat background color (hex) used when the project has no background clip.
    pub background_color: &'a str,
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait PreviewRenderer: Send + Sync {
    fn id(&self) -> &'static str;

    /// `Ok(description)` when rendering is possible, otherwise the reason.
    fn availability(&self) -> Result<String, AppError>;

    async fn render(&self, request: PreviewRequest<'_>) -> Result<(), AppError>;
}
