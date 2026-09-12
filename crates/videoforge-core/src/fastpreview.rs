//! Fast preview (P1-7): re-render `preview.mp4` from an *already generated*
//! `project.vfp.json` — skipping parsing, validation, TTS synthesis, and
//! timeline scheduling entirely — optionally on a sub-range of the timeline
//! and/or at a lower-resolution render preset (P1-6). This is the "quick
//! iteration on visuals/audio timing" half of "without waiting for a full
//! production render"; TTS (the slow step) already has its own cache
//! (`tts::TtsCache`), so a full `generate` re-run is not itself slow once
//! dialogue audio is cached — but it still re-validates, re-schedules, and
//! (without a render preset) re-renders at full resolution. This module
//! renders once, directly, from the project IR already on disk.
//!
//! A "selected scene" is expressed the same way a range is: as
//! `[start_ms, end_ms)` millisecond bounds. Turning a scene/dialogue index
//! into those bounds is left to the caller (e.g. by reading the matching
//! cue's timing out of `captions.srt` or `project.vfp.json`) — this module
//! only needs the resolved range, so it stays decoupled from any particular
//! notion of "scene."

use std::collections::BTreeMap;
use std::path::Path;

use tokio_util::sync::CancellationToken;
use videoforge_project::VideoProject;

use crate::config::Config;
use crate::error::AppError;
use crate::preset::RenderPreset;
use crate::preview::{CharacterSpriteSet, PreviewRenderer, PreviewRequest};
use crate::workspace::Workspace;

pub struct FastPreviewRequest<'a> {
    /// The already-generated project (e.g. loaded from
    /// `generated/<slug>/project.vfp.json`).
    pub project: &'a VideoProject,
    /// Directory the project's relative asset paths resolve against —
    /// normally the same `generated/<slug>/` directory it was loaded from.
    pub project_dir: &'a Path,
    pub config: &'a Config,
    pub workspace: &'a Workspace,
    pub renderer: &'a dyn PreviewRenderer,
    pub output: &'a Path,
    /// Render only `[start_ms, end_ms)` of the timeline. `None` renders the
    /// whole thing (still skipping TTS/validation/scheduling — "fast"
    /// refers to what this module skips, not only to range-limiting).
    pub range_ms: Option<(u64, u64)>,
    /// Render preset (P1-6) overriding resolution/fps/codec/quality/audio.
    /// `None` renders at the project's own `video` settings with the
    /// renderer's default encode settings.
    pub preset: Option<&'static RenderPreset>,
}

/// Render a fast preview per `request`. Character sprites are re-resolved
/// from the character manifest (cheap: no audio, no lip-sync analysis — the
/// project's `character_performance` clips already carry their lip-sync
/// file paths from the original `generate`).
pub async fn render(request: FastPreviewRequest<'_>) -> Result<(), AppError> {
    let mut project = request.project.clone();
    if let Some(preset) = request.preset {
        project.video.width = preset.width;
        project.video.height = preset.height;
        project.video.fps = preset.fps;
    }
    let encode = request.preset.map(|p| p.encode).unwrap_or_default();

    let character_sprites = resolve_character_sprites(&project, request.config, request.workspace)?;
    let font = request
        .config
        .preview
        .font
        .as_deref()
        .and_then(|f| request.workspace.resolve_allow_absolute(f).ok())
        .filter(|p| p.is_file());

    request
        .renderer
        .render(PreviewRequest {
            project: &project,
            project_dir: request.project_dir,
            output: request.output,
            font: font.as_deref(),
            background_color: &request.config.preview.background_color,
            subtitle: &request.config.preview.subtitle,
            encode: &encode,
            range_ms: request.range_ms,
            character_sprites: &character_sprites,
            cancel: CancellationToken::new(),
        })
        .await
}

fn resolve_character_sprites(
    project: &VideoProject,
    config: &Config,
    workspace: &Workspace,
) -> Result<BTreeMap<String, CharacterSpriteSet>, AppError> {
    let mut out = BTreeMap::new();
    let Some(loaded) = crate::character::load_manifest(config, workspace)? else {
        return Ok(out);
    };
    let manifest_dir = loaded.path.parent().unwrap_or_else(|| Path::new("."));
    for clip in project.character_performance_clips() {
        if out.contains_key(&clip.character) {
            continue;
        }
        let Some(character) = loaded.manifest.find(&clip.character) else {
            continue;
        };
        if let Some(sprites) = crate::character::resolve_png_sprites(character, manifest_dir)? {
            out.insert(
                clip.character.clone(),
                CharacterSpriteSet {
                    closed: sprites.closed,
                    half: sprites.half,
                    open: sprites.open,
                },
            );
        }
    }
    Ok(out)
}
