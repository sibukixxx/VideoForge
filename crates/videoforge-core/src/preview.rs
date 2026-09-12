//! Preview renderer abstraction (design §19).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use videoforge_project::VideoProject;

use crate::error::AppError;

/// Absolute paths to a `png_lipsync` character's three mouth-state sprites
/// (P0-1). Resolved by `core::generate` from the character manifest —
/// outside the project IR and outside the workspace, the same way a Live2D
/// `model.path` or `preview.font` is handled — and passed to the renderer
/// exactly like `PreviewRequest::font`: a filesystem path the renderer reads
/// directly, never copied into `generated/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSpriteSet {
    pub closed: PathBuf,
    pub half: PathBuf,
    pub open: PathBuf,
}

pub struct PreviewRequest<'a> {
    pub project: &'a VideoProject,
    /// Directory the project's relative asset paths resolve against.
    pub project_dir: &'a Path,
    pub output: &'a Path,
    /// Optional font file for captions.
    pub font: Option<&'a Path>,
    /// Flat background color (hex) used when the project has no background clip.
    pub background_color: &'a str,
    /// Resolved sprite sets for every `png_lipsync` character referenced on
    /// the project's `character_performance` track, keyed by character id.
    /// Empty for a project that uses no character, or only Live2D
    /// characters (frame rendering for those is still Phase 1, see
    /// `docs/live2d-renderer-decision.md`) — a renderer must not error on an
    /// empty map, only skip character compositing.
    pub character_sprites: &'a BTreeMap<String, CharacterSpriteSet>,
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait PreviewRenderer: Send + Sync {
    fn id(&self) -> &'static str;

    /// `Ok(description)` when rendering is possible, otherwise the reason.
    fn availability(&self) -> Result<String, AppError>;

    async fn render(&self, request: PreviewRequest<'_>) -> Result<(), AppError>;
}
