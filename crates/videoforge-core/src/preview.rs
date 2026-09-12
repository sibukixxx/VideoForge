//! Preview renderer abstraction (design §19).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use videoforge_project::VideoProject;

use crate::config::SubtitleConfig;
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

/// Video/audio encode settings a render preset (P1-6) bundles alongside
/// resolution/fps: everything the renderer's own output-side FFmpeg options
/// (`-c:v`/`-preset`/`-crf`/`-b:a`) need, decoupled from `videoforge.yaml`
/// so a preset stays a fixed, named bundle rather than exposing arbitrary
/// codec strings as a config surface. `Default` reproduces the encode
/// settings this renderer has always used, so a `generate` run with no
/// preset selected is byte-for-byte unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeSettings {
    pub video_codec: &'static str,
    /// x264's own `-preset` (encoder speed/efficiency trade-off, e.g.
    /// `veryfast`/`medium`/`ultrafast`) — named `encoder_speed` here to avoid
    /// colliding with "render preset" (`crate::preset::RenderPreset`).
    pub encoder_speed: &'static str,
    pub crf: u32,
    pub audio_bitrate_kbps: u32,
}
impl Default for EncodeSettings {
    fn default() -> Self {
        Self {
            video_codec: "libx264",
            encoder_speed: "veryfast",
            crf: 23,
            audio_bitrate_kbps: 192,
        }
    }
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
    /// Caption/subtitle styling (P1-3): position, margin, colors, outline,
    /// background box.
    pub subtitle: &'a SubtitleConfig,
    /// Encode settings (P1-6): codec/quality/audio-bitrate, normally a
    /// render preset's; `EncodeSettings::default()` when none was selected.
    pub encode: &'a EncodeSettings,
    /// Render only `[start_ms, end_ms)` of the timeline (P1-7 fast preview),
    /// via an output-side `-ss`/`-t` trim — the filter graph itself is
    /// unchanged, so every absolute-timeline expression (fades, Ken Burns,
    /// ducking windows) still means the same thing it would for a full
    /// render. `None` renders the whole timeline.
    pub range_ms: Option<(u64, u64)>,
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
