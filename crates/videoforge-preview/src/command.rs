//! Pure FFmpeg command builder (VF-051). No process is spawned here, so the
//! whole plan is unit-testable without FFmpeg installed.
//!
//! Paths inside the filter graph are kept *relative to the project directory*
//! (FFmpeg runs with `cwd = project_dir`), so a workspace path containing
//! spaces, Japanese, quotes or a drive letter never enters the graph at all.
//! Only the optional font file may be absolute; it goes through
//! [`quote_filter_value`], whose escaping rules are documented there and
//! verified against a real FFmpeg in `tests/ffmpeg_real.rs`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use videoforge_core::config::{SubtitleConfig, SubtitlePosition};
use videoforge_core::lipsync::{mouth_segments, LipSyncTrack, MouthState};
use videoforge_core::preview::{CharacterSpriteSet, PreviewRequest};
use videoforge_core::project::{Clip, FitMode, Transform, VideoProject};
use videoforge_core::AppError;

pub const AUDIO_SAMPLE_RATE: u32 = 48000;
pub const FADE_SECS: f64 = 0.3;
/// A `png_lipsync` character sprite is scaled to this fraction of the frame
/// height at `presentation.scale == 1.0` (P0-1). Hard-coded but named, same
/// reasoning as the lip-sync amplitude thresholds in `core::lipsync`.
pub const CHARACTER_BASE_HEIGHT_FRACTION: f64 = 0.62;

/// Default fade-in/fade-out length for a visual clip's `"fade"` intent
/// (P1-5) when the script didn't say `intent_duration_ms`.
pub const DEFAULT_INTENT_FADE_MS: u64 = 400;
/// Ken Burns zoom-in amount for the `"zoom"` intent: the visible crop
/// shrinks from 100% to `1.0 - ZOOM_IN_FRACTION` of the source over the
/// clip's own duration (P1-5).
pub const ZOOM_IN_FRACTION: f64 = 0.15;
/// Crop window size for the `"slide"` (pan) intent, as a fraction of the
/// source; the window slides across the remaining `1.0 - PAN_CROP_FRACTION`
/// of the source over the clip's own duration (P1-5).
pub const PAN_CROP_FRACTION: f64 = 0.9;
/// BGM volume multiplier applied under any overlapping dialogue (P1-4).
/// Multiplies the clip's own `volume`, so at `volume: 1.0` this drops BGM
/// to 35% while someone is speaking, then back to full between lines.
pub const BGM_DUCK_VOLUME: f32 = 0.35;

/// One character's overlay plan: where its sprites go, and when `half`/
/// `open` should cover the always-present `closed` base layer. `closed` has
/// no window list because it is the base layer for the character's entire
/// on-screen presence — the same reason an inactive speaker in the P0-1
/// acceptance scenario reads as "closed", not "absent".
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterOverlayPlan {
    pub character: String,
    pub closed: PathBuf,
    pub half: PathBuf,
    pub open: PathBuf,
    pub transform: Transform,
    /// Timeline windows (ms) where the half-open sprite covers `closed`.
    pub half_windows: Vec<(u64, u64)>,
    /// Timeline windows (ms) where the fully-open sprite covers `closed`.
    pub open_windows: Vec<(u64, u64)>,
}

/// Read every performance clip's lip-sync curve for each sprite-resolved
/// character and turn it into overlay windows. The only filesystem access
/// in this module — mirrors `RenderPlan::write_caption_files` being the only
/// filesystem access for captions; `RenderPlan::build` itself stays pure.
pub fn build_character_overlays(
    project: &VideoProject,
    project_dir: &Path,
    sprites: &BTreeMap<String, CharacterSpriteSet>,
) -> Result<Vec<CharacterOverlayPlan>, AppError> {
    let mut overlays = Vec::with_capacity(sprites.len());
    for (character_id, sprite_set) in sprites {
        let clips: Vec<_> = project
            .character_performance_clips()
            .into_iter()
            .filter(|c| &c.character == character_id)
            .collect();
        let Some(first) = clips.first() else {
            continue;
        };
        let mut half_windows = Vec::new();
        let mut open_windows = Vec::new();
        for clip in &clips {
            let path = clip.lip_sync.resolve(project_dir);
            let bytes = std::fs::read(&path).map_err(|e| AppError::read(&path, e))?;
            let track: LipSyncTrack = serde_json::from_slice(&bytes)
                .map_err(|e| AppError::serialization(path.display().to_string(), e))?;
            for seg in mouth_segments(&track, clip.start_ms, clip.duration_ms) {
                match seg.state {
                    MouthState::Half => half_windows.push((seg.start_ms, seg.end_ms)),
                    MouthState::Open => open_windows.push((seg.start_ms, seg.end_ms)),
                    MouthState::Closed => {}
                }
            }
        }
        overlays.push(CharacterOverlayPlan {
            character: character_id.clone(),
            closed: sprite_set.closed.clone(),
            half: sprite_set.half.clone(),
            open: sprite_set.open.clone(),
            transform: first.transform,
            half_windows,
            open_windows,
        });
    }
    Ok(overlays)
}

/// What kind of source a [`VisualLayerPlan`] reads from — affects only the
/// FFmpeg *input* options (`-loop`, `-stream_loop`), not the filter chain
/// applied to it, which is identical for every layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualLayerSource {
    /// A still image, character stand-in, or background — looped
    /// (`-loop 1`) so it covers the layer's whole `duration_ms`.
    Image,
    /// A general video clip (P1-2).
    Video {
        trim_start_ms: u64,
        /// Repeat the source (`-stream_loop -1`) to fill `duration_ms`.
        looping: bool,
    },
}

/// One general visual clip (`Image`/`Character` stand-in/`Video`) placed on
/// the timeline (P1-1/P1-2), sharing one compositing implementation instead
/// of a special renderer per clip kind. Distinct from
/// [`CharacterOverlayPlan`] (P0-1's closed/half/open mouth-swap), which
/// stays its own code path.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualLayerPlan {
    /// Relative to the project dir (unlike `CharacterOverlayPlan`'s sprite
    /// paths, which live outside it) — the same rule every other project
    /// asset path already follows.
    pub path: String,
    pub source: VisualLayerSource,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub transform: Transform,
    /// `Presentation::intent`, e.g. `"fade"`/`"slide"`/`"zoom"`/`"cut"` (P1-5).
    pub intent: Option<String>,
    pub intent_duration_ms: Option<u64>,
}

/// Collect every `Image`/`Character` (stand-in)/`Video` clip into one
/// ordered list of layers, sorted by `Transform::layer` (stable, so clips on
/// the same layer keep their original track/script order) — pure, no
/// filesystem access, unlike `build_character_overlays` (which must read
/// each lip-sync curve off disk).
pub fn build_visual_layers(project: &VideoProject) -> Vec<VisualLayerPlan> {
    let mut layers: Vec<VisualLayerPlan> = project
        .clips()
        .filter_map(|clip| match clip {
            Clip::Image(c) => Some(VisualLayerPlan {
                path: c.source.as_str().to_string(),
                source: VisualLayerSource::Image,
                start_ms: c.start_ms,
                duration_ms: c.duration_ms,
                transform: c.transform,
                intent: c.presentation.as_ref().and_then(|p| p.intent.clone()),
                intent_duration_ms: c.presentation.as_ref().and_then(|p| p.intent_duration_ms),
            }),
            Clip::Character(c) => Some(VisualLayerPlan {
                path: c.source.as_str().to_string(),
                source: VisualLayerSource::Image,
                start_ms: c.start_ms,
                duration_ms: c.duration_ms,
                transform: c.transform,
                intent: c.presentation.as_ref().and_then(|p| p.intent.clone()),
                intent_duration_ms: c.presentation.as_ref().and_then(|p| p.intent_duration_ms),
            }),
            Clip::Video(c) => Some(VisualLayerPlan {
                path: c.source.as_str().to_string(),
                source: VisualLayerSource::Video {
                    trim_start_ms: c.trim_start_ms,
                    looping: c.looping,
                },
                start_ms: c.start_ms,
                duration_ms: c.duration_ms,
                transform: c.transform,
                intent: c.presentation.as_ref().and_then(|p| p.intent.clone()),
                intent_duration_ms: c.presentation.as_ref().and_then(|p| p.intent_duration_ms),
            }),
            _ => None,
        })
        .collect();
    layers.sort_by_key(|l| l.transform.layer);
    layers
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPlan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
    pub text: String,
    /// Relative to the project dir.
    pub text_file: String,
    pub speaker_file: String,
    /// Resolved caption text color (P1-3): `CaptionClip::color` (a
    /// per-speaker `SpeakerConfig::caption_color` override, baked into the
    /// IR at generate time) if set, else `preview.subtitle.font_color`.
    pub color: String,
}

/// One on-screen text clip (P1-1's `TextClip`) ready to render via
/// `drawtext`, positioned by the same `Transform` every visual clip uses
/// instead of the fixed caption band `CaptionPlan` always renders in.
#[derive(Debug, Clone, PartialEq)]
pub struct TextPlan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub transform: Transform,
    /// Hex color, always populated (defaults applied here, not at render time).
    pub color: String,
    /// Relative to the project dir.
    pub text_file: String,
}

/// One background music clip, ready for the audio engine (P1-4): volume,
/// fade-in/out, loop, trim and normalization all land in `audio_filter`.
#[derive(Debug, Clone, PartialEq)]
pub struct BgmPlan {
    /// Relative to the project dir.
    pub path: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub looping: bool,
    pub trim_start_ms: u64,
    pub fade_in_ms: u64,
    pub fade_out_ms: u64,
    pub normalize: bool,
}

/// One sound-effect clip (P1-4): a one-shot, volume only — trim/loop/fade
/// don't apply to something meant to play once, in full, as authored.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundEffectPlan {
    /// Relative to the project dir.
    pub path: String,
    pub start_ms: u64,
    pub volume: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub total_ms: u64,
    /// Relative background image path, if any.
    pub background: Option<String>,
    pub background_color: String,
    /// Relative dialogue audio paths, their start offset, and their
    /// duration (the last is needed to compute BGM ducking windows).
    pub audio: Vec<(String, u64, u64)>,
    pub captions: Vec<CaptionPlan>,
    /// On-screen text clips (P1-1), distinct from `captions`.
    pub texts: Vec<TextPlan>,
    /// Background music clips (P1-4).
    pub bgm: Vec<BgmPlan>,
    /// Sound-effect clips (P1-4).
    pub sound_effects: Vec<SoundEffectPlan>,
    pub font: Option<PathBuf>,
    pub output: PathBuf,
    /// Absolute scratch directory (caption text files are written here).
    pub scratch_dir: PathBuf,
    /// `scratch_dir` relative to the project dir.
    pub scratch_rel: String,
    /// `png_lipsync` character overlays (P0-1). Empty for a project that
    /// uses no character, or only Live2D ones.
    pub character_overlays: Vec<CharacterOverlayPlan>,
    /// General `Image`/`Character`-stand-in/`Video` clips (P1-1/P1-2),
    /// sorted by `Transform::layer`. Composited *before* `character_overlays`
    /// (backdrops/props sit behind a performing character) and well before
    /// captions/texts (always on top — P1-3's caption/character layout rule
    /// extended to every visual clip, not just characters).
    pub visual_layers: Vec<VisualLayerPlan>,
    /// Caption/subtitle styling (P1-3): position, margin, colors, outline,
    /// background box, font scale.
    pub subtitle: SubtitleConfig,
}

impl RenderPlan {
    pub fn from_request(
        request: &PreviewRequest<'_>,
        scratch_dir: &Path,
    ) -> Result<Self, AppError> {
        let scratch_rel = relative_to(scratch_dir, request.project_dir).ok_or_else(|| {
            AppError::PreviewRenderFailed(format!(
                "scratch dir {} must be inside the project dir {}",
                scratch_dir.display(),
                request.project_dir.display()
            ))
        })?;
        let character_overlays = build_character_overlays(
            request.project,
            request.project_dir,
            request.character_sprites,
        )?;
        Ok(Self::build(
            request.project,
            request.background_color,
            request.subtitle.clone(),
            request.font.map(Path::to_path_buf),
            request.output.to_path_buf(),
            scratch_dir.to_path_buf(),
            scratch_rel,
            character_overlays,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build(
        project: &VideoProject,
        background_color: &str,
        subtitle: SubtitleConfig,
        font: Option<PathBuf>,
        output: PathBuf,
        scratch_dir: PathBuf,
        scratch_rel: String,
        character_overlays: Vec<CharacterOverlayPlan>,
    ) -> Self {
        let background = project
            .background_clips()
            .first()
            .map(|b| b.source.as_str().to_string());
        let audio = project
            .audio_clips()
            .iter()
            .map(|a| (a.source.as_str().to_string(), a.start_ms, a.duration_ms))
            .collect();
        let captions = project
            .caption_clips()
            .iter()
            .enumerate()
            .map(|(i, c)| CaptionPlan {
                start_ms: c.start_ms,
                end_ms: c.start_ms + c.duration_ms,
                speaker: c
                    .speaker_display
                    .clone()
                    .unwrap_or_else(|| c.speaker.clone()),
                text: c.text.clone(),
                text_file: format!("{scratch_rel}/caption-{:03}.txt", i + 1),
                speaker_file: format!("{scratch_rel}/speaker-{:03}.txt", i + 1),
                color: c
                    .color
                    .clone()
                    .unwrap_or_else(|| subtitle.font_color.clone()),
            })
            .collect();
        const DEFAULT_TEXT_COLOR: &str = "white";
        let texts = project
            .text_clips()
            .iter()
            .enumerate()
            .map(|(i, c)| TextPlan {
                start_ms: c.start_ms,
                end_ms: c.start_ms + c.duration_ms,
                text: c.text.clone(),
                transform: c.transform,
                color: c.color.clone().unwrap_or_else(|| DEFAULT_TEXT_COLOR.into()),
                text_file: format!("{scratch_rel}/text-{:03}.txt", i + 1),
            })
            .collect();
        let visual_layers = build_visual_layers(project);
        let bgm = project
            .bgm_clips()
            .iter()
            .map(|b| BgmPlan {
                path: b.source.as_str().to_string(),
                start_ms: b.start_ms,
                duration_ms: b.duration_ms,
                volume: b.volume,
                looping: b.looping,
                trim_start_ms: b.trim_start_ms,
                fade_in_ms: b.fade_in_ms,
                fade_out_ms: b.fade_out_ms,
                normalize: b.normalize,
            })
            .collect();
        let sound_effects = project
            .sound_effect_clips()
            .iter()
            .map(|s| SoundEffectPlan {
                path: s.source.as_str().to_string(),
                start_ms: s.start_ms,
                volume: s.volume,
            })
            .collect();
        Self {
            width: project.video.width,
            height: project.video.height,
            fps: project.video.fps.max(1),
            total_ms: project.total_duration_ms().max(1000),
            background,
            background_color: normalize_color(background_color),
            audio,
            captions,
            texts,
            bgm,
            sound_effects,
            font,
            output,
            scratch_dir,
            scratch_rel,
            character_overlays,
            visual_layers,
            subtitle,
        }
    }

    pub fn write_caption_files(&self) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.scratch_dir)
            .map_err(|e| AppError::write(&self.scratch_dir, e))?;
        for (i, c) in self.captions.iter().enumerate() {
            let text = self.scratch_dir.join(format!("caption-{:03}.txt", i + 1));
            std::fs::write(&text, wrap_text(&c.text, self.max_chars_per_line()))
                .map_err(|e| AppError::write(&text, e))?;
            let speaker = self.scratch_dir.join(format!("speaker-{:03}.txt", i + 1));
            std::fs::write(&speaker, &c.speaker).map_err(|e| AppError::write(&speaker, e))?;
        }
        for (i, t) in self.texts.iter().enumerate() {
            let path = self.scratch_dir.join(format!("text-{:03}.txt", i + 1));
            std::fs::write(&path, &t.text).map_err(|e| AppError::write(&path, e))?;
        }
        Ok(())
    }

    fn caption_font_size(&self) -> u32 {
        (self.height as f64 * 0.048 * self.subtitle.font_scale as f64).round() as u32
    }

    fn speaker_font_size(&self) -> u32 {
        (self.height as f64 * 0.034 * self.subtitle.font_scale as f64).round() as u32
    }

    fn max_chars_per_line(&self) -> usize {
        // CJK glyphs are ~1em wide; leave 10% margins.
        let usable = self.width as f64 * 0.8;
        (usable / self.caption_font_size() as f64).floor().max(8.0) as usize
    }

    /// Pixel distance between `subtitle.position`'s edge and the caption
    /// text's baseline (`preview.subtitle.margin_fraction` of frame height —
    /// `0.20` was a hard-coded constant here before P1-3).
    fn caption_margin_px(&self) -> u32 {
        (self.height as f64 * self.subtitle.margin_fraction as f64).round() as u32
    }

    /// Pixel height of the "caption safe area" (speaker label and caption
    /// text, plus their margins), measured from whichever edge
    /// `subtitle.position` anchors captions to. Shared by the caption
    /// `drawtext` y-position and the character overlay clamp below, so a
    /// character can never be placed under the captions — structural, not
    /// incidental (P0-1: "字幕との重なりを考慮できる構造").
    fn caption_safe_area_px(&self) -> u32 {
        self.caption_margin_px() + self.speaker_font_size() + 16
    }

    /// `drawtext` `y=` expression for the caption text, and for the speaker
    /// label above it — "above" meaning further from the frame's edge,
    /// regardless of whether captions render at the top or the bottom.
    fn caption_y_exprs(&self) -> (String, String) {
        let margin = self.caption_margin_px();
        let speaker_size = self.speaker_font_size();
        match self.subtitle.position {
            SubtitlePosition::Bottom => (
                format!("h-{margin}"),
                format!("h-{}", margin + speaker_size + 16),
            ),
            SubtitlePosition::Top => (
                format!("{}", margin + speaker_size + 16),
                format!("{margin}"),
            ),
        }
    }

    /// Target pixel height for a character sprite at the given
    /// `presentation.scale`, clamped so it can never be taller than the area
    /// above the caption safe zone (P0-1: "frame外にはみ出さない" for the
    /// vertical axis; see `overlay_position_exprs` for the horizontal one).
    fn character_target_height_px(&self, scale: f32) -> u32 {
        let max_h = self
            .height
            .saturating_sub(self.caption_safe_area_px())
            .max(8);
        let raw = (self.height as f64 * CHARACTER_BASE_HEIGHT_FRACTION * scale.max(0.0) as f64)
            .round() as u32;
        raw.clamp(8, max_h)
    }

    /// FFmpeg `overlay` x/y expressions placing a character at `transform`'s
    /// normalized centre (same semantics as every other clip's `Transform`),
    /// clamped to stay fully inside the frame and clear of the caption safe
    /// area on whichever edge `subtitle.position` anchors it to.
    /// `overlay_w`/`overlay_h` are resolved by FFmpeg at run time from the
    /// actual scaled sprite, so this does not need to know pixel sizes.
    fn overlay_position_exprs(&self, transform: &Transform) -> (String, String) {
        let safe_area = self.caption_safe_area_px();
        let x = format!(
            "min(max(0,{:.4}*main_w-overlay_w/2),main_w-overlay_w)",
            transform.x
        );
        let y = match self.subtitle.position {
            SubtitlePosition::Bottom => format!(
                "min(max(0,{:.4}*main_h-overlay_h/2),main_h-{safe_area}-overlay_h)",
                transform.y
            ),
            SubtitlePosition::Top => format!(
                "min(max({safe_area},{:.4}*main_h-overlay_h/2),main_h-overlay_h)",
                transform.y
            ),
        };
        (x, y)
    }

    /// One `overlay` filter's `enable` value from a set of timeline windows,
    /// merged with `+` (FFmpeg's boolean OR) — mirrors how caption `enable`
    /// windows are built, just with more than one interval.
    fn enable_windows_expr(windows: &[(u64, u64)]) -> String {
        windows
            .iter()
            .map(|(s, e)| format!("between(t,{},{})", ms_to_secs(*s), ms_to_secs(*e)))
            .collect::<Vec<_>>()
            .join("+")
    }

    /// FFmpeg `overlay` x/y expressions for a general visual clip — same
    /// normalized-centre placement and frame-bounds clamp as
    /// `overlay_position_exprs`, but *without* the caption-safe-area
    /// subtraction: a full-frame diagram or background image is allowed to
    /// occupy the whole frame (captions still draw on top of it regardless,
    /// since they are composited last).
    fn visual_position_exprs(transform: &Transform) -> (String, String) {
        let x = format!(
            "min(max(0,{:.4}*main_w-overlay_w/2),main_w-overlay_w)",
            transform.x
        );
        let y = format!(
            "min(max(0,{:.4}*main_h-overlay_h/2),main_h-overlay_h)",
            transform.y
        );
        (x, y)
    }

    /// The box a general visual clip's `fit` scales into: the frame itself
    /// times `Transform::scale` (`scale == 1.0` covers/contains the whole
    /// frame, matching `FitMode`'s own doc comment — "the frame" is this
    /// box, not literally the output canvas).
    fn visual_layer_target_size(&self, scale: f32) -> (u32, u32) {
        let w = ((self.width as f64) * scale.max(0.0) as f64)
            .round()
            .max(2.0) as u32;
        let h = ((self.height as f64) * scale.max(0.0) as f64)
            .round()
            .max(2.0) as u32;
        (w, h)
    }

    /// Filter implementing one `FitMode` into a `w`x`h` box. `None` for
    /// `FitMode::None` — no filter at all, i.e. the source's native pixel
    /// size, exactly matching its doc comment.
    fn fit_filter(fit: FitMode, w: u32, h: u32) -> Option<String> {
        match fit {
            FitMode::Contain => Some(format!(
                "scale={w}:{h}:force_original_aspect_ratio=decrease"
            )),
            FitMode::Cover => Some(format!(
                "scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"
            )),
            FitMode::Stretch => Some(format!("scale={w}:{h}")),
            FitMode::None => None,
        }
    }

    /// `Transform::crop`'s static (time-invariant) crop of the source,
    /// applied before `fit`/`scale` per its own doc comment.
    fn static_crop_filter(crop: &videoforge_core::project::CropRect) -> String {
        format!(
            "crop=w='iw*{:.4}':h='ih*{:.4}':x='iw*{:.4}':y='ih*{:.4}'",
            crop.width, crop.height, crop.x, crop.y
        )
    }

    /// Ken Burns pan (`"slide"`) / zoom (`"zoom"`) as a *time-varying* crop
    /// window (`eval=frame`, referencing the filter's own `t`) — deliberately
    /// not the `zoompan` filter, whose internal frame counter has no relation
    /// to this graph's shared absolute timeline (every layer's `enable`
    /// window, and this expression, both key off the same `t`). Still images
    /// only (P1-5 scope: "don't build a complex editor" — Ken Burns is
    /// specifically a still-image effect in every mainstream editor too);
    /// `None` for any other intent, including on a video layer.
    fn ken_burns_filter(intent: &str, start_ms: u64, duration_ms: u64) -> Option<String> {
        let start_sec = fmt_secs(start_ms as f64 / 1000.0);
        let dur_sec = fmt_secs((duration_ms.max(1)) as f64 / 1000.0);
        let progress = format!("clip((t-{start_sec})/{dur_sec}\\,0\\,1)");
        match intent {
            "zoom" => Some(format!(
                "crop=w='iw*(1-{zf}*{progress})':h='ih*(1-{zf}*{progress})':x='(in_w-out_w)/2':y='(in_h-out_h)/2':eval=frame",
                zf = ZOOM_IN_FRACTION,
            )),
            "slide" => Some(format!(
                "crop=w='iw*{pf}':h='ih*{pf}':x='(in_w-out_w)*{progress}':y='(in_h-out_h)/2':eval=frame",
                pf = PAN_CROP_FRACTION,
            )),
            _ => None,
        }
    }

    /// `"fade"` intent: an alpha ramp in at the clip's own start and out at
    /// its own end, each `intent_duration_ms` long (default
    /// `DEFAULT_INTENT_FADE_MS`, capped to half the clip's own duration so
    /// the two ramps never overlap).
    fn fade_filters(
        start_ms: u64,
        duration_ms: u64,
        intent_duration_ms: Option<u64>,
    ) -> Vec<String> {
        let fade_ms = intent_duration_ms
            .unwrap_or(DEFAULT_INTENT_FADE_MS)
            .clamp(1, (duration_ms / 2).max(1));
        let start_sec = start_ms as f64 / 1000.0;
        let end_sec = (start_ms + duration_ms) as f64 / 1000.0;
        let fade_sec = fade_ms as f64 / 1000.0;
        vec![
            "format=rgba".to_string(),
            format!(
                "fade=t=in:st={}:d={}:alpha=1",
                fmt_secs(start_sec),
                fmt_secs(fade_sec)
            ),
            format!(
                "fade=t=out:st={}:d={}:alpha=1",
                fmt_secs((end_sec - fade_sec).max(start_sec)),
                fmt_secs(fade_sec)
            ),
        ]
    }

    /// `Transform::rotation_deg`, clockwise around the centre, transparent
    /// fill outside the rotated bounds (`c=none`) — the canvas expands to
    /// `rotw`/`roth` so `overlay_w`/`overlay_h` (used for positioning) stay
    /// correct after rotation.
    fn rotation_filters(rotation_deg: f32) -> Vec<String> {
        if rotation_deg == 0.0 {
            return Vec::new();
        }
        let rad = format!("{:.6}*PI/180", rotation_deg);
        vec![
            "format=rgba".to_string(),
            format!("rotate={rad}:ow=rotw({rad}):oh=roth({rad}):c=none"),
        ]
    }

    /// `Transform::opacity` as a constant alpha-channel multiplier.
    fn opacity_filters(opacity: f32) -> Vec<String> {
        if opacity >= 1.0 {
            return Vec::new();
        }
        vec![
            "format=rgba".to_string(),
            format!("colorchannelmixer=aa={:.4}", opacity.clamp(0.0, 1.0)),
        ]
    }

    /// A video layer's source-trim + timeline-placement step: bounds the
    /// source to exactly `[trim_start_ms, trim_start_ms + duration_ms)`
    /// (the upper bound matters for a looping source, whose input is
    /// otherwise infinite — `-stream_loop -1`), then shifts its
    /// (zero-based, post-trim) presentation timestamps so they land on this
    /// clip's own absolute position on the shared timeline. This is what
    /// makes a video layer's `t` mean the same thing as an image layer's —
    /// both equal absolute timeline seconds — so `fade_filters`/
    /// `ken_burns_filter` (image-only) and the overlay `enable` window all
    /// key off one consistent clock.
    fn video_trim_filter(trim_start_ms: u64, start_ms: u64, duration_ms: u64) -> String {
        let trim_start_sec = fmt_secs(trim_start_ms as f64 / 1000.0);
        let trim_end_sec = fmt_secs((trim_start_ms + duration_ms) as f64 / 1000.0);
        let offset_sec = fmt_secs(start_ms as f64 / 1000.0);
        format!(
            "trim=start={trim_start_sec}:end={trim_end_sec},setpts=PTS-STARTPTS+{offset_sec}/TB"
        )
    }

    /// Full per-layer filter chain, in application order: video trim/offset
    /// (video only) → static crop → Ken Burns pan/zoom (images only) → fit
    /// into this layer's target box → rotation → opacity → fade.
    fn visual_layer_chain(&self, layer: &VisualLayerPlan) -> Vec<String> {
        let mut chain = Vec::new();
        if let VisualLayerSource::Video { trim_start_ms, .. } = &layer.source {
            chain.push(Self::video_trim_filter(
                *trim_start_ms,
                layer.start_ms,
                layer.duration_ms,
            ));
        }
        if let Some(crop) = &layer.transform.crop {
            chain.push(Self::static_crop_filter(crop));
        }
        if matches!(layer.source, VisualLayerSource::Image) {
            if let Some(intent) = &layer.intent {
                if let Some(kb) = Self::ken_burns_filter(intent, layer.start_ms, layer.duration_ms)
                {
                    chain.push(kb);
                }
            }
        }
        let (w, h) = self.visual_layer_target_size(layer.transform.scale);
        if let Some(fit) = Self::fit_filter(layer.transform.fit, w, h) {
            chain.push(fit);
        }
        chain.extend(Self::rotation_filters(layer.transform.rotation_deg));
        chain.extend(Self::opacity_filters(layer.transform.opacity));
        if layer.intent.as_deref() == Some("fade") {
            chain.extend(Self::fade_filters(
                layer.start_ms,
                layer.duration_ms,
                layer.intent_duration_ms,
            ));
        }
        chain
    }

    /// General visual clip (`Image`/`Character` stand-in/`Video`) overlay
    /// chain (P1-1/P1-2/P1-5). Returns the filters to append and the label
    /// the next stage (character overlays, then captions/texts) should read
    /// from — `input_label` unchanged when there are no visual layers.
    fn visual_layer_filters(&self, input_label: &str) -> (Vec<String>, String) {
        let mut filters = Vec::new();
        let mut current = input_label.to_string();
        for (i, layer) in self.visual_layers.iter().enumerate() {
            let input_index = 1 + self.audio.len() + i;
            let chain = self.visual_layer_chain(layer);
            let scaled = format!("vis{i}");
            if chain.is_empty() {
                filters.push(format!("[{input_index}:v]null[{scaled}]"));
            } else {
                filters.push(format!("[{input_index}:v]{}[{scaled}]", chain.join(",")));
            }
            let (x, y) = Self::visual_position_exprs(&layer.transform);
            let enable = format!(
                "between(t,{},{})",
                ms_to_secs(layer.start_ms),
                ms_to_secs(layer.start_ms + layer.duration_ms)
            );
            let next = format!("vis{i}out");
            filters.push(format!(
                "[{current}][{scaled}]overlay=x={x}:y={y}:enable='{enable}':eof_action=repeat[{next}]"
            ));
            current = next;
        }
        (filters, current)
    }

    /// Every visual layer's FFmpeg input options, in the exact order
    /// `visual_layer_filters` assigns input indices to them — shared by
    /// `build_args` so the two never drift apart.
    pub fn visual_layer_inputs(&self) -> Vec<(String, bool, bool)> {
        // (path, loop_image, stream_loop_video) — see `build_args`.
        self.visual_layers
            .iter()
            .map(|l| match l.source {
                VisualLayerSource::Image => (l.path.clone(), true, false),
                VisualLayerSource::Video { looping, .. } => (l.path.clone(), false, looping),
            })
            .collect()
    }

    /// Character overlay filter chain: `closed` is always on for the
    /// character's full on-screen presence (an inactive speaker's default
    /// pose); `half`/`open` cover it only during their amplitude-derived
    /// windows. Returns the filters to append and the label the next stage
    /// (captions/fade) should read from — `input_label` unchanged when there
    /// are no character overlays at all.
    fn character_filters(&self, input_label: &str) -> (Vec<String>, String) {
        let mut filters = Vec::new();
        let mut current = input_label.to_string();
        for (i, ov) in self.character_overlays.iter().enumerate() {
            let target_h = self.character_target_height_px(ov.transform.scale);
            let (x, y) = self.overlay_position_exprs(&ov.transform);
            let base_input = 1 + self.audio.len() + self.visual_layers.len() + i * 3;
            let layer = |filters: &mut Vec<String>,
                         current: &mut String,
                         suffix: &str,
                         input_index: usize,
                         enable: Option<&str>| {
                let scaled = format!("cov{i}{suffix}");
                filters.push(format!("[{input_index}:v]scale=-2:{target_h}[{scaled}]"));
                let next = format!("cov{i}{suffix}out");
                let enable_clause = enable.map(|e| format!(":enable='{e}'")).unwrap_or_default();
                filters.push(format!(
                    "[{current}][{scaled}]overlay=x={x}:y={y}{enable_clause}[{next}]"
                ));
                *current = next;
            };
            layer(&mut filters, &mut current, "closed", base_input, None);
            if !ov.half_windows.is_empty() {
                let enable = Self::enable_windows_expr(&ov.half_windows);
                layer(
                    &mut filters,
                    &mut current,
                    "half",
                    base_input + 1,
                    Some(&enable),
                );
            }
            if !ov.open_windows.is_empty() {
                let enable = Self::enable_windows_expr(&ov.open_windows);
                layer(
                    &mut filters,
                    &mut current,
                    "open",
                    base_input + 2,
                    Some(&enable),
                );
            }
        }
        (filters, current)
    }

    /// Every character overlay's sprite paths, in the exact order
    /// `character_filters` assigns FFmpeg input indices to them — shared by
    /// `build_args` so the two never drift apart.
    pub fn character_input_paths(&self) -> Vec<&Path> {
        self.character_overlays
            .iter()
            .flat_map(|ov| [ov.closed.as_path(), ov.half.as_path(), ov.open.as_path()])
            .collect()
    }

    /// Full `-filter_complex` video graph. When there are no character
    /// overlays this is exactly the single `[0:v]...[v]` chain it has always
    /// been; character overlays (P0-1) turn it into a small multi-node graph
    /// — background, then each character composited on top in order, then
    /// the same caption/fade chain applied last so captions always draw over
    /// a character, never under it (P0-1: "字幕との重なりを考慮できる構造").
    pub fn video_filter(&self) -> String {
        let (w, h) = (self.width, self.height);
        let base = [
            format!("scale={w}:{h}:force_original_aspect_ratio=decrease"),
            format!("pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"),
            "setsar=1".to_string(),
            "format=yuv420p".to_string(),
        ];

        let font = self
            .font
            .as_ref()
            .map(|f| format!(":fontfile={}", quote_filter_value(&f.to_string_lossy())))
            .unwrap_or_default();
        let caption_size = self.caption_font_size();
        let speaker_size = self.speaker_font_size();
        let (caption_y, speaker_y) = self.caption_y_exprs();
        let caption_box = if self.subtitle.background {
            format!(
                ":box=1:boxcolor={}:boxborderw=10",
                self.subtitle.background_color
            )
        } else {
            String::new()
        };
        let mut tail = Vec::new();
        for c in &self.captions {
            let enable = format!(
                "enable='between(t,{},{})'",
                ms_to_secs(c.start_ms),
                ms_to_secs(c.end_ms)
            );
            tail.push(format!(
                "drawtext=textfile={}{font}:fontsize={speaker_size}:fontcolor=white:box=1:boxcolor=0x000000AA:boxborderw=10:x=(w-text_w)/2:y={speaker_y}:{enable}",
                quote_filter_value(&c.speaker_file)
            ));
            tail.push(format!(
                "drawtext=textfile={}{font}:fontsize={caption_size}:fontcolor={}:borderw={}:bordercolor={}:line_spacing=8:text_align=center:x=(w-text_w)/2:y={caption_y}{caption_box}:{enable}",
                quote_filter_value(&c.text_file),
                c.color,
                self.subtitle.outline_width,
                self.subtitle.outline_color,
            ));
        }
        // On-screen text clips (P1-1/P1-3): rendered after captions, so a
        // title/label is never hidden behind one, positioned by its own
        // Transform instead of the fixed caption band.
        for t in &self.texts {
            let enable = format!(
                "enable='between(t,{},{})'",
                ms_to_secs(t.start_ms),
                ms_to_secs(t.end_ms)
            );
            let font_size = ((h as f64 * 0.05) * t.transform.scale.max(0.0) as f64)
                .round()
                .max(1.0) as u32;
            tail.push(format!(
                "drawtext=textfile={}{font}:fontsize={font_size}:fontcolor={}:borderw=2:bordercolor=black:x={:.4}*w-text_w/2:y={:.4}*h-text_h/2:{enable}",
                quote_filter_value(&t.text_file),
                t.color,
                t.transform.x,
                t.transform.y,
            ));
        }
        let total = self.total_ms as f64 / 1000.0;
        tail.push(format!("fade=t=in:st=0:d={FADE_SECS}"));
        tail.push(format!(
            "fade=t=out:st={}:d={FADE_SECS}",
            fmt_secs((total - FADE_SECS).max(0.0))
        ));

        if self.character_overlays.is_empty() && self.visual_layers.is_empty() {
            let chain: Vec<String> = base.into_iter().chain(tail).collect();
            return format!("[0:v]{}[v]", chain.join(","));
        }

        let mut parts = vec![format!("[0:v]{}[bg0]", base.join(","))];
        let (visual_filters, post_visual_label) = self.visual_layer_filters("bg0");
        parts.extend(visual_filters);
        let (overlay_filters, post_overlay_label) = self.character_filters(&post_visual_label);
        parts.extend(overlay_filters);
        parts.push(format!("[{post_overlay_label}]{}[v]", tail.join(",")));
        parts.join(";")
    }

    /// First FFmpeg input index of the BGM clips — after the background,
    /// dialogue audio, visual layers and character sprites (all of which
    /// `:a` never needs to reference), so adding BGM/SE inputs here can
    /// never shift any `:v` index computed elsewhere.
    fn bgm_input_start(&self) -> usize {
        1 + self.audio.len() + self.visual_layers.len() + self.character_overlays.len() * 3
    }

    fn se_input_start(&self) -> usize {
        self.bgm_input_start() + self.bgm.len()
    }

    /// Every BGM/SE clip's FFmpeg input options, in the exact order
    /// `audio_filter` assigns input indices to them — shared by
    /// `build_args` so the two never drift apart. `stream_loop` is `-1`
    /// only for a looping BGM clip (bounded back down to its own
    /// `duration_ms` by `audio_filter`'s `atrim`, same reasoning as a
    /// looping `Video` layer).
    pub fn bgm_and_se_inputs(&self) -> Vec<(String, bool)> {
        self.bgm
            .iter()
            .map(|b| (b.path.clone(), b.looping))
            .chain(self.sound_effects.iter().map(|s| (s.path.clone(), false)))
            .collect()
    }

    /// Dialogue windows `(start_ms, end_ms)`, used to duck BGM under
    /// overlapping speech (P1-4).
    fn dialogue_windows(&self) -> Vec<(u64, u64)> {
        self.audio
            .iter()
            .map(|(_, start, duration)| (*start, start + duration))
            .collect()
    }

    /// One BGM clip's filter chain: optional trim (bounding a looping
    /// source back down to its own `duration_ms`, so `-stream_loop -1`
    /// input still terminates), resample/format, `adelay` to its timeline
    /// position, optional loudness normalization, its base `volume`,
    /// optional fade in/out, and — the P1-4 headline feature — ducking
    /// under any overlapping dialogue via a single time-varying `volume`
    /// expression rather than one `volume` filter per dialogue line.
    fn bgm_chain(&self, b: &BgmPlan) -> Vec<String> {
        let mut chain = Vec::new();
        if b.looping {
            chain.push(format!(
                "atrim=start={}:end={}",
                fmt_secs(b.trim_start_ms as f64 / 1000.0),
                fmt_secs((b.trim_start_ms + b.duration_ms) as f64 / 1000.0)
            ));
        } else if b.trim_start_ms > 0 {
            chain.push(format!(
                "atrim=start={}",
                fmt_secs(b.trim_start_ms as f64 / 1000.0)
            ));
        }
        chain.push(format!("aresample={AUDIO_SAMPLE_RATE}"));
        chain.push("aformat=channel_layouts=stereo".to_string());
        chain.push(format!("adelay={}:all=1", b.start_ms));
        if b.normalize {
            chain.push("dynaudnorm".to_string());
        }
        chain.push(format!("volume={:.4}", b.volume.max(0.0)));
        if b.fade_in_ms > 0 {
            chain.push(format!(
                "afade=t=in:st={}:d={}",
                fmt_secs(b.start_ms as f64 / 1000.0),
                fmt_secs(b.fade_in_ms as f64 / 1000.0)
            ));
        }
        if b.fade_out_ms > 0 {
            let end_sec = (b.start_ms + b.duration_ms) as f64 / 1000.0;
            let fade_sec = b.fade_out_ms as f64 / 1000.0;
            chain.push(format!(
                "afade=t=out:st={}:d={}",
                fmt_secs((end_sec - fade_sec).max(b.start_ms as f64 / 1000.0)),
                fmt_secs(fade_sec)
            ));
        }
        let dialogue_windows = self.dialogue_windows();
        if !dialogue_windows.is_empty() {
            let duck_expr = dialogue_windows
                .iter()
                .map(|(s, e)| format!("between(t,{},{})", ms_to_secs(*s), ms_to_secs(*e)))
                .collect::<Vec<_>>()
                .join("+");
            chain.push(format!(
                "volume=eval=frame:volume='if({duck_expr}\\,{:.4}\\,1)'",
                BGM_DUCK_VOLUME
            ));
        }
        chain
    }

    pub fn audio_filter(&self) -> String {
        let mut parts = Vec::new();
        let mut labels = Vec::new();

        for (i, (_, start, _)) in self.audio.iter().enumerate() {
            let label = format!("[a{}]", i + 1);
            parts.push(format!(
                "[{}:a]aresample={AUDIO_SAMPLE_RATE},aformat=channel_layouts=stereo,adelay={start}:all=1{label}",
                i + 1
            ));
            labels.push(label);
        }

        let bgm_start = self.bgm_input_start();
        for (i, b) in self.bgm.iter().enumerate() {
            let input_index = bgm_start + i;
            let label = format!("[bgm{i}]");
            let chain = self.bgm_chain(b);
            parts.push(format!("[{input_index}:a]{}{label}", chain.join(",")));
            labels.push(label);
        }

        let se_start = self.se_input_start();
        for (i, s) in self.sound_effects.iter().enumerate() {
            let input_index = se_start + i;
            let label = format!("[se{i}]");
            parts.push(format!(
                "[{input_index}:a]aresample={AUDIO_SAMPLE_RATE},aformat=channel_layouts=stereo,adelay={}:all=1,volume={:.4}{label}",
                s.start_ms,
                s.volume.max(0.0)
            ));
            labels.push(label);
        }

        if labels.is_empty() {
            parts.push(format!("anullsrc=r={AUDIO_SAMPLE_RATE}:cl=stereo[aout]"));
        } else if labels.len() == 1 {
            parts.push(format!("{}anull[aout]", labels[0]));
        } else {
            parts.push(format!(
                "{}amix=inputs={}:duration=longest:normalize=0[aout]",
                labels.join(""),
                labels.len()
            ));
        }
        parts.join(";")
    }
}

pub fn build_args(plan: &RenderPlan) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];

    // input 0: background
    match &plan.background {
        Some(bg) => {
            args.extend(
                ["-loop", "1", "-framerate", &plan.fps.to_string(), "-i", bg].map(OsString::from),
            );
        }
        None => {
            args.extend(
                [
                    "-f",
                    "lavfi",
                    "-i",
                    &format!(
                        "color=c={}:s={}x{}:r={}",
                        plan.background_color, plan.width, plan.height, plan.fps
                    ),
                ]
                .map(OsString::from),
            );
        }
    }
    // inputs 1..N: audio
    for (path, _, _) in &plan.audio {
        args.push("-i".into());
        args.push(path.into());
    }
    // next inputs: general visual layers (Image/Character stand-in/Video,
    // P1-1/P1-2), relative to the project dir — order must match
    // `RenderPlan::visual_layer_filters`'s input-index math.
    for (path, loop_image, stream_loop_video) in plan.visual_layer_inputs() {
        if loop_image {
            args.extend(
                [
                    "-loop",
                    "1",
                    "-framerate",
                    &plan.fps.to_string(),
                    "-i",
                    &path,
                ]
                .map(OsString::from),
            );
        } else if stream_loop_video {
            args.extend(["-stream_loop", "-1", "-i", &path].map(OsString::from));
        } else {
            args.extend(["-i", &path].map(OsString::from));
        }
    }
    // remaining inputs: character sprites (closed/half/open per character,
    // P0-1), absolute paths — like the font, these live outside the project
    // dir and outside the filter graph string, so no escaping is needed;
    // order must match `RenderPlan::character_filters`'s input-index math.
    for path in plan.character_input_paths() {
        args.extend(
            [
                "-loop",
                "1",
                "-framerate",
                &plan.fps.to_string(),
                "-i",
                &path.to_string_lossy(),
            ]
            .map(OsString::from),
        );
    }
    // final inputs: BGM/SE (P1-4), audio-only — order must match
    // `RenderPlan::bgm_input_start`/`se_input_start`'s index math. A looping
    // BGM clip uses `-stream_loop -1`, bounded back down to its own
    // `duration_ms` by `audio_filter`'s `atrim` (same reasoning as a
    // looping `Video` layer).
    for (path, looping) in plan.bgm_and_se_inputs() {
        if looping {
            args.extend(["-stream_loop", "-1", "-i", &path].map(OsString::from));
        } else {
            args.extend(["-i", &path].map(OsString::from));
        }
    }

    let filter = format!("{};{}", plan.video_filter(), plan.audio_filter());
    args.extend(["-filter_complex", &filter, "-map", "[v]", "-map", "[aout]"].map(OsString::from));
    args.extend(
        [
            "-t",
            &fmt_secs(plan.total_ms as f64 / 1000.0),
            "-r",
            &plan.fps.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ]
        .map(OsString::from),
    );
    args.push(plan.output.clone().into());
    args
}

fn ms_to_secs(ms: u64) -> String {
    fmt_secs(ms as f64 / 1000.0)
}

fn fmt_secs(s: f64) -> String {
    let t = format!("{s:.3}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() {
        "0".into()
    } else {
        t.to_string()
    }
}

/// `#1e1e2e` → `0x1e1e2e`; names pass through.
fn normalize_color(c: &str) -> String {
    let c = c.trim();
    match c.strip_prefix('#') {
        Some(hex) => format!("0x{hex}"),
        None => c.to_string(),
    }
}

/// Quote a value (a path, typically) for use as a filtergraph option value,
/// e.g. `drawtext=textfile=<here>`.
///
/// FFmpeg parses a `-filter_complex` string in two passes, each with its own
/// escaping (see "Notes on filtergraph escaping" in `ffmpeg-filters`):
///
/// 1. the **graph** pass splits filters on `[],;` and filter options on `=`;
///    `'...'` protects everything (including `\`) until the next `'`;
/// 2. the **option** pass then splits the surviving text on `:`; here `\x`
///    yields `x`, and leading/trailing whitespace is trimmed unless escaped.
///
/// So the value is first escaped for pass 2 ([`escape_option_value`]) and the
/// result is then quoted for pass 1 ([`quote_graph_token`]). A single level —
/// quoting alone — loses every `'` and `:` in the second pass.
///
/// | input        | output            |
/// |--------------|-------------------|
/// | `a b`        | `'a b'`           |
/// | `it's`       | `'it\'\''s'`      |
/// | `C:\x\y.ttf`  | `'C\:\\x\\y.ttf'` |
/// | `a,b;[c]=d`  | `'a,b;[c]=d'`     |
///
/// Backslashes are kept (escaped), not rewritten to `/`: both are valid path
/// separators on Windows, and a `\` inside a Unix file name must survive.
pub fn quote_filter_value(value: &str) -> String {
    quote_graph_token(&escape_option_value(value))
}

/// Option-pass (second level) escaping: backslash before `\`, `'` and `:`,
/// and before leading/trailing whitespace so it is not trimmed.
pub fn escape_option_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let last = chars.len().saturating_sub(1);
    let mut out = String::with_capacity(value.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let needs_escape =
            matches!(c, '\\' | '\'' | ':') || (c.is_ascii_whitespace() && (i == 0 || i == last));
        if needs_escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Graph-pass (first level) quoting: wrap in `'...'`, with each embedded `'`
/// written as `'\''` (close, escaped quote, reopen).
pub fn quote_graph_token(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Naive wrapping for caption text: hard-wrap lines longer than `max_chars`
/// (counted in chars, CJK-friendly), keeping explicit newlines.
pub fn wrap_text(text: &str, max_chars: usize) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() <= max_chars {
            out.push(line.to_string());
            continue;
        }
        for chunk in chars.chunks(max_chars.max(1)) {
            out.push(chunk.iter().collect());
        }
    }
    out.join("\n")
}

fn relative_to(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use videoforge_core::project::{
        AudioClip, BackgroundClip, BgmClip, CaptionClip, CharacterClip, Clip, CropRect, FitMode,
        ImageClip, Presentation, RelativeAssetPath, SoundEffectClip, TextClip, Track, TrackKind,
        VideoClip, VideoSettings,
    };

    fn project(with_background: bool) -> VideoProject {
        let mut p = VideoProject::new("s", "S", VideoSettings::default());
        if with_background {
            p.tracks.push(Track {
                id: "bg".into(),
                kind: TrackKind::Background,
                clips: vec![Clip::Background(BackgroundClip {
                    id: "bg-1".into(),
                    source: RelativeAssetPath::new("assets/background/default.png").unwrap(),
                    start_ms: 0,
                    duration_ms: 7290,
                    extra: BTreeMap::new(),
                })],
            });
        }
        p.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![
                Clip::Audio(AudioClip {
                    id: "a1".into(),
                    source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                    start_ms: 0,
                    duration_ms: 3410,
                    speaker: "reimu".into(),
                    extra: BTreeMap::new(),
                }),
                Clip::Audio(AudioClip {
                    id: "a2".into(),
                    source: RelativeAssetPath::new("assets/audio/002.wav").unwrap(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    extra: BTreeMap::new(),
                }),
            ],
        });
        p.tracks.push(Track {
            id: "caption".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "c1".into(),
                    text: "こんにちは".into(),
                    start_ms: 0,
                    duration_ms: 3410,
                    speaker: "reimu".into(),
                    speaker_display: Some("霊夢".into()),
                    color: None,
                    extra: BTreeMap::new(),
                }),
                Clip::Caption(CaptionClip {
                    id: "c2".into(),
                    text: "やあ".into(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    speaker_display: None,
                    color: None,
                    extra: BTreeMap::new(),
                }),
            ],
        });
        p
    }

    fn plan(with_background: bool, font: Option<&str>) -> RenderPlan {
        RenderPlan::build(
            &project(with_background),
            "#1e1e2e",
            SubtitleConfig::default(),
            font.map(PathBuf::from),
            PathBuf::from("/out/preview.mp4"),
            PathBuf::from("/proj/.preview.mp4.tmp"),
            ".preview.mp4.tmp".into(),
            Vec::new(),
        )
    }

    #[test]
    fn builds_command_with_image_background() {
        let args = build_args(&plan(true, None));
        let s: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&s[..3], &["-hide_banner", "-nostdin", "-y"]);
        assert!(s.contains(&"-loop".to_string()));
        assert!(s.contains(&"assets/background/default.png".to_string()));
        assert!(s.contains(&"assets/audio/002.wav".to_string()));
        let fc = &s[s.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(fc.starts_with("[0:v]scale=1920:1080"));
        assert!(fc.contains("adelay=3610:all=1[a2]"));
        assert!(fc.contains("amix=inputs=2:duration=longest:normalize=0[aout]"));
        assert!(fc.contains("enable='between(t,3.61,7.29)'"));
        assert!(fc.contains("textfile='.preview.mp4.tmp/caption-002.txt'"));
        assert!(fc.contains("fade=t=out:st=6.99:d=0.3"));
        let t = &s[s.iter().position(|x| x == "-t").unwrap() + 1];
        assert_eq!(t, "7.29");
        assert_eq!(s.last().unwrap(), "/out/preview.mp4");
    }

    #[test]
    fn flat_color_when_no_background() {
        let args = build_args(&plan(false, Some(r"C:\Windows\Fonts\meiryo.ttc")));
        let s: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(s.contains(&"lavfi".to_string()));
        assert!(s.iter().any(|a| a == "color=c=0x1e1e2e:s=1920x1080:r=30"));
        let fc = &s[s.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(
            fc.contains("fontfile='C\\:\\\\Windows\\\\Fonts\\\\meiryo.ttc'"),
            "{fc}"
        );
    }

    #[test]
    fn single_audio_uses_anull() {
        let mut p = project(false);
        p.tracks[0].clips.truncate(1);
        let plan = RenderPlan::build(
            &p,
            "black",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        );
        assert!(plan.audio_filter().ends_with("[a1]anull[aout]"));
        assert_eq!(plan.background_color, "black");
    }

    #[test]
    fn writes_caption_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = plan(false, None);
        plan.scratch_dir = dir.path().join("scratch");
        plan.write_caption_files().unwrap();
        assert_eq!(
            std::fs::read_to_string(plan.scratch_dir.join("speaker-001.txt")).unwrap(),
            "霊夢"
        );
        assert_eq!(
            std::fs::read_to_string(plan.scratch_dir.join("speaker-002.txt")).unwrap(),
            "marisa"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(fmt_secs(3.41), "3.41");
        assert_eq!(fmt_secs(0.0), "0");
        assert_eq!(fmt_secs(7.0), "7");
        assert_eq!(
            wrap_text("あいうえおかきくけこ", 4),
            "あいうえ\nおかきく\nけこ"
        );
        assert_eq!(wrap_text("ab\ncd", 10), "ab\ncd");
    }

    /// One row per character class from issue #12. The expected strings are
    /// what a real FFmpeg accepts — see `tests/ffmpeg_real.rs`.
    #[test]
    fn filter_value_escaping_per_character_class() {
        let cases: &[(&str, &str)] = &[
            ("plain.txt", "'plain.txt'"),
            ("with space.txt", "'with space.txt'"),
            ("日本語 字幕.txt", "'日本語 字幕.txt'"),
            ("it's.txt", "'it\\'\\''s.txt'"),
            ("co:lon.txt", "'co\\:lon.txt'"),
            ("com,ma.txt", "'com,ma.txt'"),
            ("semi;colon.txt", "'semi;colon.txt'"),
            ("br[ack]ets.txt", "'br[ack]ets.txt'"),
            ("eq=ual.txt", "'eq=ual.txt'"),
            ("back\\slash.txt", "'back\\\\slash.txt'"),
            (" lead.txt", "'\\ lead.txt'"),
            ("trail ", "'trail\\ '"),
            ("pct%.txt", "'pct%.txt'"),
            // Windows drive path: the drive colon is escaped, separators kept.
            (
                r"C:\Users\霊夢\Fonts\it's.ttf",
                "'C\\:\\\\Users\\\\霊夢\\\\Fonts\\\\it\\'\\''s.ttf'",
            ),
            // Forward slashes on Windows need no escaping at all.
            (
                "C:/Windows/Fonts/meiryo.ttc",
                "'C\\:/Windows/Fonts/meiryo.ttc'",
            ),
            // UNC path: only backslashes to escape.
            (
                r"\\server\share\fonts\a.ttf",
                "'\\\\\\\\server\\\\share\\\\fonts\\\\a.ttf'",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(&quote_filter_value(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn escaping_levels_compose() {
        assert_eq!(escape_option_value("a:b'c\\d"), "a\\:b\\'c\\\\d");
        assert_eq!(quote_graph_token("a'b"), "'a'\\''b'");
        assert_eq!(
            quote_filter_value("it's"),
            quote_graph_token(&escape_option_value("it's"))
        );
    }

    /// Special characters in the *workspace* path never reach the graph: every
    /// path inside it is relative to the project dir (FFmpeg's cwd).
    #[test]
    fn workspace_path_stays_out_of_the_filter_graph() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\Users\霊夢\my videos\it's [v2]; a,b")
        } else {
            PathBuf::from("/home/霊夢/my videos/it's [v2]; a,b")
        };
        let scratch = root.join(".preview.mp4.tmp");
        let plan = RenderPlan::build(
            &project(true),
            "#000000",
            SubtitleConfig::default(),
            None,
            root.join("preview.mp4"),
            scratch.clone(),
            relative_to(&scratch, &root).unwrap(),
            Vec::new(),
        );
        assert_eq!(plan.scratch_rel, ".preview.mp4.tmp");
        let args: Vec<String> = build_args(&plan)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let fc = &args[args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(!fc.contains("霊夢"), "{fc}");
        assert!(!fc.contains("my videos"), "{fc}");
        assert!(
            fc.contains("textfile='.preview.mp4.tmp/caption-001.txt'"),
            "{fc}"
        );
        // The output path is a plain argument, not part of the graph.
        assert_eq!(
            args.last().unwrap(),
            &root.join("preview.mp4").to_string_lossy()
        );
        // Background and audio inputs are plain relative arguments too.
        assert!(args.contains(&"assets/background/default.png".to_string()));
    }

    // ------------------------------------------------------- character overlays (P0-1)

    fn character_overlay(
        name: &str,
        x: f32,
        half_windows: Vec<(u64, u64)>,
        open_windows: Vec<(u64, u64)>,
    ) -> CharacterOverlayPlan {
        CharacterOverlayPlan {
            character: name.into(),
            closed: PathBuf::from(format!("/sprites/{name}/closed.png")),
            half: PathBuf::from(format!("/sprites/{name}/half.png")),
            open: PathBuf::from(format!("/sprites/{name}/open.png")),
            transform: Transform {
                x,
                y: 0.8,
                scale: 1.0,
                ..Transform::default()
            },
            half_windows,
            open_windows,
        }
    }

    fn plan_with_overlays(overlays: Vec<CharacterOverlayPlan>) -> RenderPlan {
        RenderPlan::build(
            &project(false),
            "#000000",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            overlays,
        )
    }

    #[test]
    fn video_filter_with_no_overlays_is_the_original_single_chain() {
        let plan = plan_with_overlays(Vec::new());
        let fc = plan.video_filter();
        assert!(fc.starts_with("[0:v]scale="));
        assert!(fc.ends_with("[v]"));
        assert!(!fc.contains("overlay="));
    }

    #[test]
    fn video_filter_composites_character_before_captions() {
        let overlay = character_overlay("a", 0.2, vec![(100, 200)], vec![(200, 300)]);
        let plan = plan_with_overlays(vec![overlay]);
        let fc = plan.video_filter();

        // background scaled onto its own label, not directly into the tail chain
        assert!(fc.contains("[0:v]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p[bg0]"));
        // closed layer has no `enable` — always on
        assert!(fc.contains("overlay=x=") && fc.contains("[cov0closedout]"));
        let closed_overlay_stmt = fc
            .split(';')
            .find(|s| s.contains("[cov0closed]") && s.contains("overlay="))
            .unwrap();
        assert!(
            !closed_overlay_stmt.contains("enable="),
            "{closed_overlay_stmt}"
        );
        // half/open layers are time-windowed
        assert!(fc.contains("enable='between(t,0.1,0.2)'"));
        assert!(fc.contains("enable='between(t,0.2,0.3)'"));
        // captions/fade apply to the post-overlay label, so they draw on top
        assert!(fc.contains("[cov0openout]drawtext="));
        assert!(fc.ends_with("[v]"));
    }

    #[test]
    fn video_filter_merges_multiple_windows_with_plus() {
        let overlay = character_overlay("a", 0.5, vec![(0, 50), (100, 150)], Vec::new());
        let plan = plan_with_overlays(vec![overlay]);
        let fc = plan.video_filter();
        assert!(
            fc.contains("enable='between(t,0,0.05)+between(t,0.1,0.15)'"),
            "{fc}"
        );
    }

    #[test]
    fn video_filter_omits_half_or_open_layer_when_its_window_list_is_empty() {
        let overlay = character_overlay("a", 0.5, Vec::new(), vec![(0, 50)]);
        let plan = plan_with_overlays(vec![overlay]);
        let fc = plan.video_filter();
        assert!(!fc.contains("cov0half"), "{fc}");
        assert!(fc.contains("cov0open"), "{fc}");
    }

    #[test]
    fn character_input_paths_orders_closed_half_open_per_character_in_order() {
        let plan = plan_with_overlays(vec![
            character_overlay("a", 0.2, vec![], vec![]),
            character_overlay("b", 0.8, vec![], vec![]),
        ]);
        let paths: Vec<String> = plan
            .character_input_paths()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            paths,
            vec![
                "/sprites/a/closed.png",
                "/sprites/a/half.png",
                "/sprites/a/open.png",
                "/sprites/b/closed.png",
                "/sprites/b/half.png",
                "/sprites/b/open.png",
            ]
        );
    }

    #[test]
    fn build_args_places_character_inputs_after_audio_inputs_at_the_indices_the_graph_expects() {
        // `project(false)` has 2 audio clips → inputs 0 (background) 1,2
        // (audio) → character sprites start at input 3.
        let plan = plan_with_overlays(vec![character_overlay("a", 0.5, vec![(0, 50)], vec![])]);
        let args: Vec<String> = build_args(&plan)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args.iter()
                .filter(|a| a.as_str() == "/sprites/a/closed.png")
                .count(),
            1
        );
        let fc = &args[args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("[3:v]scale=-2:"), "{fc}"); // closed
        assert!(fc.contains("[4:v]scale=-2:"), "{fc}"); // half
        assert!(!fc.contains("[5:v]scale=-2:"), "{fc}"); // open window is empty, no `open` layer
    }

    #[test]
    fn overlay_position_clamps_and_uses_normalized_transform() {
        let plan = plan_with_overlays(Vec::new());
        let (x, y) = plan.overlay_position_exprs(&Transform {
            x: 0.5,
            y: 0.9,
            ..Transform::default()
        });
        assert!(x.contains("0.5000*main_w"));
        assert!(y.contains("0.9000*main_h"));
        assert!(y.contains("main_h-")); // clamped against the caption safe area
    }

    // -------------------------------------------------------------- subtitle engine (P1-3)

    fn plan_with_subtitle(subtitle: SubtitleConfig) -> RenderPlan {
        RenderPlan::build(
            &project(false),
            "#000000",
            subtitle,
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        )
    }

    fn plan_with_subtitle_and_caption_colors(
        subtitle: SubtitleConfig,
        colors: [Option<&str>; 2],
    ) -> RenderPlan {
        let mut p = project(false);
        let caption_track = p
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Caption)
            .unwrap();
        for (clip, color) in caption_track.clips.iter_mut().zip(colors) {
            if let Clip::Caption(c) = clip {
                c.color = color.map(str::to_string);
            }
        }
        RenderPlan::build(
            &p,
            "#000000",
            subtitle,
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        )
    }

    #[test]
    fn subtitle_position_bottom_is_the_default_and_anchors_to_the_bottom_edge() {
        let plan = plan_with_subtitle(SubtitleConfig::default());
        let (caption_y, speaker_y) = plan.caption_y_exprs();
        assert!(caption_y.starts_with("h-"), "{caption_y}");
        assert!(speaker_y.starts_with("h-"), "{speaker_y}");
    }

    #[test]
    fn subtitle_position_top_anchors_captions_to_the_top_edge_with_the_speaker_label_above() {
        let subtitle = SubtitleConfig {
            position: SubtitlePosition::Top,
            ..SubtitleConfig::default()
        };
        let plan = plan_with_subtitle(subtitle);
        let (caption_y, speaker_y) = plan.caption_y_exprs();
        assert!(!caption_y.starts_with("h-"), "{caption_y}");
        assert!(!speaker_y.starts_with("h-"), "{speaker_y}");
        let caption_px: u32 = caption_y.parse().unwrap();
        let speaker_px: u32 = speaker_y.parse().unwrap();
        assert!(
            speaker_px < caption_px,
            "speaker label must stay closer to the top edge than the caption text"
        );
    }

    #[test]
    fn subtitle_position_top_clamps_character_overlays_below_the_top_safe_area() {
        let subtitle = SubtitleConfig {
            position: SubtitlePosition::Top,
            ..SubtitleConfig::default()
        };
        let plan = plan_with_subtitle(subtitle);
        let safe_area = plan.caption_safe_area_px();
        let (_, y) = plan.overlay_position_exprs(&Transform::default());
        assert!(
            y.contains(&format!("max({safe_area}")),
            "{y} must clamp its minimum to the top safe area"
        );
    }

    #[test]
    fn subtitle_style_colors_outline_and_background_box_are_applied() {
        let subtitle = SubtitleConfig {
            font_color: "yellow".into(),
            outline_color: "blue".into(),
            outline_width: 7,
            background: true,
            background_color: "0x112233CC".into(),
            ..SubtitleConfig::default()
        };
        let plan = plan_with_subtitle(subtitle);
        let fc = plan.video_filter();
        assert!(fc.contains("fontcolor=yellow"), "{fc}");
        assert!(fc.contains("bordercolor=blue"), "{fc}");
        assert!(fc.contains("borderw=7"), "{fc}");
        assert!(fc.contains("boxcolor=0x112233CC"), "{fc}");
    }

    #[test]
    fn subtitle_background_box_is_off_by_default() {
        let plan = plan_with_subtitle(SubtitleConfig::default());
        let fc = plan.video_filter();
        // one `box=1` per caption's always-on speaker-name label; the
        // caption text itself gets no box unless `subtitle.background` opts in.
        assert_eq!(fc.matches("box=1").count(), plan.captions.len());
    }

    #[test]
    fn caption_color_override_falls_back_to_subtitle_font_color() {
        let subtitle = SubtitleConfig {
            font_color: "white".into(),
            ..SubtitleConfig::default()
        };
        let plan = plan_with_subtitle_and_caption_colors(subtitle, [Some("#ff00ff"), None]);
        assert_eq!(plan.captions[0].color, "#ff00ff");
        assert_eq!(plan.captions[1].color, "white");
        let fc = plan.video_filter();
        assert!(fc.contains("fontcolor=#ff00ff"), "{fc}");
        assert!(fc.contains("fontcolor=white"), "{fc}");
    }

    #[test]
    fn subtitle_font_scale_grows_caption_font_and_shrinks_the_wrap_width() {
        let default_plan = plan_with_subtitle(SubtitleConfig::default());
        let scaled = SubtitleConfig {
            font_scale: 2.0,
            ..SubtitleConfig::default()
        };
        let scaled_plan = plan_with_subtitle(scaled);
        assert!(scaled_plan.caption_font_size() > default_plan.caption_font_size());
        assert!(scaled_plan.max_chars_per_line() < default_plan.max_chars_per_line());
    }

    #[test]
    fn subtitle_margin_fraction_changes_the_caption_safe_area() {
        let small_plan = plan_with_subtitle(SubtitleConfig {
            margin_fraction: 0.05,
            ..SubtitleConfig::default()
        });
        let large_plan = plan_with_subtitle(SubtitleConfig {
            margin_fraction: 0.35,
            ..SubtitleConfig::default()
        });
        assert!(small_plan.caption_safe_area_px() < large_plan.caption_safe_area_px());
    }

    #[test]
    fn character_target_height_scales_with_presentation_scale_and_is_clamped() {
        let plan = plan_with_overlays(Vec::new());
        let base = plan.character_target_height_px(1.0);
        let half = plan.character_target_height_px(0.5);
        assert!(half < base);
        // absurd scale never exceeds the area above the caption safe zone
        let huge = plan.character_target_height_px(100.0);
        assert!(huge <= plan.height - plan.caption_safe_area_px());
        // zero/negative scale never collapses to 0 (a visible sprite, however small)
        assert!(plan.character_target_height_px(0.0) >= 8);
    }

    #[test]
    fn build_character_overlays_reads_lipsync_curves_into_windows() {
        use videoforge_core::lipsync::LipSyncTrack;
        use videoforge_core::preview::CharacterSpriteSet;

        let dir = tempfile::tempdir().unwrap();
        let lipsync_dir = dir.path().join("assets/character/mock_a");
        std::fs::create_dir_all(&lipsync_dir).unwrap();
        let track = LipSyncTrack {
            interval_ms: 50,
            samples: vec![
                videoforge_core::lipsync::LipSyncSample {
                    t_ms: 0,
                    mouth_open: 0.0,
                },
                videoforge_core::lipsync::LipSyncSample {
                    t_ms: 50,
                    mouth_open: 0.9,
                },
            ],
        };
        std::fs::write(
            lipsync_dir.join("lipsync-001.json"),
            track.to_json().unwrap(),
        )
        .unwrap();

        let mut p = project(false);
        p.tracks.push(Track {
            id: "character_performance".into(),
            kind: TrackKind::CharacterPerformance,
            clips: vec![Clip::CharacterPerformance(
                videoforge_core::project::CharacterPerformanceClip {
                    id: "cp-001".into(),
                    start_ms: 1000,
                    duration_ms: 100,
                    character: "mock_a".into(),
                    expression: "default".into(),
                    motion: "idle".into(),
                    lip_sync: RelativeAssetPath::new("assets/character/mock_a/lipsync-001.json")
                        .unwrap(),
                    transform: Transform {
                        x: 0.2,
                        ..Transform::default()
                    },
                    extra: BTreeMap::new(),
                },
            )],
        });

        let mut sprites = BTreeMap::new();
        sprites.insert(
            "mock_a".to_string(),
            CharacterSpriteSet {
                closed: PathBuf::from("/sprites/closed.png"),
                half: PathBuf::from("/sprites/half.png"),
                open: PathBuf::from("/sprites/open.png"),
            },
        );

        let overlays = build_character_overlays(&p, dir.path(), &sprites).unwrap();
        assert_eq!(overlays.len(), 1);
        let ov = &overlays[0];
        assert_eq!(ov.character, "mock_a");
        assert_eq!(ov.transform.x, 0.2);
        assert!(ov.half_windows.is_empty());
        // sample at 1050ms (clip start 1000 + t_ms 50) with mouth_open 0.9 → open
        assert_eq!(ov.open_windows, vec![(1050, 1100)]);
    }

    // --------------------------------------------------- visual layers (P1-1/P1-2/P1-5)

    fn image_clip(id: &str, path: &str, layer: i32, presentation: Option<Presentation>) -> Clip {
        Clip::Image(ImageClip {
            id: id.into(),
            source: RelativeAssetPath::new(path).unwrap(),
            start_ms: 0,
            duration_ms: 2000,
            transform: Transform {
                layer,
                ..Transform::default()
            },
            presentation,
            extra: BTreeMap::new(),
        })
    }

    fn video_clip(id: &str, path: &str) -> VideoClip {
        VideoClip {
            id: id.into(),
            source: RelativeAssetPath::new(path).unwrap(),
            start_ms: 1000,
            duration_ms: 3000,
            trim_start_ms: 500,
            volume: 1.0,
            muted: false,
            looping: false,
            transform: Transform::default(),
            presentation: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn build_visual_layers_sorts_by_transform_layer_stably() {
        let mut p = project(false);
        p.tracks.push(Track {
            id: "image".into(),
            kind: TrackKind::Image,
            clips: vec![
                image_clip("image-001", "assets/image/back.png", 5, None),
                image_clip("image-002", "assets/image/front.png", -1, None),
            ],
        });
        p.tracks.push(Track {
            id: "character".into(),
            kind: TrackKind::Character,
            clips: vec![Clip::Character(CharacterClip {
                id: "character-001".into(),
                source: RelativeAssetPath::new("assets/character/reimu/default.png").unwrap(),
                start_ms: 0,
                duration_ms: 2000,
                speaker: Some("reimu".into()),
                transform: Transform {
                    layer: 0,
                    ..Transform::default()
                },
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "video".into(),
            kind: TrackKind::Video,
            clips: vec![Clip::Video(video_clip(
                "video-001",
                "assets/video/clip.mp4",
            ))],
        });

        let layers = build_visual_layers(&p);
        let paths: Vec<&str> = layers.iter().map(|l| l.path.as_str()).collect();
        // sorted by layer: -1 (image-002), 0 (character-001) then default 0
        // (video-001, VideoClip::transform default layer 0, stable so it
        // keeps its original relative position after character-001), 5 (image-001)
        assert_eq!(
            paths,
            vec![
                "assets/image/front.png",
                "assets/character/reimu/default.png",
                "assets/video/clip.mp4",
                "assets/image/back.png",
            ]
        );
    }

    fn plan_with_visual_layers(layers: Vec<VisualLayerPlan>) -> RenderPlan {
        let mut plan = RenderPlan::build(
            &project(false),
            "#000000",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        );
        plan.visual_layers = layers;
        plan
    }

    fn image_layer(
        path: &str,
        start_ms: u64,
        duration_ms: u64,
        transform: Transform,
        intent: Option<&str>,
        intent_duration_ms: Option<u64>,
    ) -> VisualLayerPlan {
        VisualLayerPlan {
            path: path.into(),
            source: VisualLayerSource::Image,
            start_ms,
            duration_ms,
            transform,
            intent: intent.map(str::to_string),
            intent_duration_ms,
        }
    }

    #[test]
    fn video_filter_composites_an_image_layer_with_fit_and_position() {
        let layer = image_layer(
            "assets/image/a.png",
            500,
            1000,
            Transform {
                x: 0.25,
                y: 0.75,
                scale: 0.5,
                fit: FitMode::Cover,
                ..Transform::default()
            },
            None,
            None,
        );
        let plan = plan_with_visual_layers(vec![layer]);
        let fc = plan.video_filter();
        assert!(fc.contains("[3:v]"), "{fc}"); // input 0=bg,1,2=audio(2 clips),3=layer
        assert!(
            fc.contains("scale=960:540:force_original_aspect_ratio=increase,crop=960:540"),
            "{fc}"
        );
        assert!(fc.contains("overlay=x=min(max(0,0.2500*main_w-overlay_w/2),main_w-overlay_w)"));
        assert!(fc.contains("enable='between(t,0.5,1.5)'"));
        assert!(fc.contains("[vis0out]"));
    }

    #[test]
    fn video_filter_applies_static_crop_before_fit() {
        let layer = image_layer(
            "assets/image/a.png",
            0,
            1000,
            Transform {
                crop: Some(CropRect {
                    x: 0.1,
                    y: 0.2,
                    width: 0.5,
                    height: 0.6,
                }),
                ..Transform::default()
            },
            None,
            None,
        );
        let plan = plan_with_visual_layers(vec![layer]);
        let fc = plan.video_filter();
        assert!(
            fc.contains("crop=w='iw*0.5000':h='ih*0.6000':x='iw*0.1000':y='ih*0.2000'"),
            "{fc}"
        );
        // crop must come before the fit scale within the same layer's chain
        // (the background's own base chain also contains "scale=1920:1080",
        // so search for the fit scale that immediately follows the crop).
        let crop_pos = fc.find("crop=w='iw*0.5000'").unwrap();
        let scale_pos = fc[crop_pos..].find("scale=1920:1080").unwrap() + crop_pos;
        assert!(crop_pos < scale_pos, "{fc}");
    }

    #[test]
    fn video_filter_fade_intent_ramps_alpha_at_clip_boundaries() {
        let layer = image_layer(
            "assets/image/a.png",
            1000,
            2000,
            Transform::default(),
            Some("fade"),
            Some(500),
        );
        let plan = plan_with_visual_layers(vec![layer]);
        let fc = plan.video_filter();
        assert!(fc.contains("fade=t=in:st=1:d=0.5:alpha=1"), "{fc}");
        assert!(fc.contains("fade=t=out:st=2.5:d=0.5:alpha=1"), "{fc}");
    }

    #[test]
    fn video_filter_zoom_intent_only_applies_to_image_layers() {
        let image = image_layer(
            "assets/image/a.png",
            0,
            2000,
            Transform::default(),
            Some("zoom"),
            None,
        );
        let mut video = video_clip("video-001", "assets/video/clip.mp4");
        video.presentation = Some(Presentation {
            role: None,
            intent: Some("zoom".into()),
            intent_duration_ms: None,
        });
        let video_layer = VisualLayerPlan {
            path: video.source.as_str().to_string(),
            source: VisualLayerSource::Video {
                trim_start_ms: video.trim_start_ms,
                looping: video.looping,
            },
            start_ms: video.start_ms,
            duration_ms: video.duration_ms,
            transform: video.transform,
            intent: video.presentation.as_ref().and_then(|p| p.intent.clone()),
            intent_duration_ms: None,
        };

        let plan_image = plan_with_visual_layers(vec![image]);
        let fc_image = plan_image.video_filter();
        assert!(fc_image.contains("eval=frame"), "{fc_image}");

        let plan_video = plan_with_visual_layers(vec![video_layer]);
        let fc_video = plan_video.video_filter();
        assert!(!fc_video.contains("eval=frame"), "{fc_video}");
    }

    #[test]
    fn video_filter_video_layer_gets_trim_and_absolute_setpts_offset() {
        let layer = VisualLayerPlan {
            path: "assets/video/clip.mp4".into(),
            source: VisualLayerSource::Video {
                trim_start_ms: 500,
                looping: false,
            },
            start_ms: 2000,
            duration_ms: 3000,
            transform: Transform::default(),
            intent: None,
            intent_duration_ms: None,
        };
        let plan = plan_with_visual_layers(vec![layer]);
        let fc = plan.video_filter();
        assert!(
            fc.contains("trim=start=0.5:end=3.5,setpts=PTS-STARTPTS+2/TB"),
            "{fc}"
        );
    }

    #[test]
    fn build_args_places_visual_layer_inputs_before_character_sprites() {
        let image = image_layer(
            "assets/image/a.png",
            0,
            1000,
            Transform::default(),
            None,
            None,
        );
        let video_layer = VisualLayerPlan {
            path: "assets/video/clip.mp4".into(),
            source: VisualLayerSource::Video {
                trim_start_ms: 0,
                looping: true,
            },
            start_ms: 0,
            duration_ms: 1000,
            transform: Transform::default(),
            intent: None,
            intent_duration_ms: None,
        };
        let plan = plan_with_visual_layers(vec![image, video_layer]);
        let args: Vec<String> = build_args(&plan)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // project(false) has 2 audio clips -> inputs 0(bg),1,2(audio),3(image),4(video)
        let image_i = args.iter().position(|a| a == "assets/image/a.png").unwrap();
        assert_eq!(args[image_i - 5], "-loop");
        let video_i = args
            .iter()
            .position(|a| a == "assets/video/clip.mp4")
            .unwrap();
        assert_eq!(args[video_i - 3], "-stream_loop");
        assert!(video_i > image_i);
        let fc = &args[args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        assert!(fc.contains("[3:v]"), "{fc}");
        assert!(fc.contains("[4:v]"), "{fc}");
    }

    #[test]
    fn video_filter_renders_text_clips_after_captions() {
        let mut p = project(false);
        p.tracks.push(Track {
            id: "text".into(),
            kind: TrackKind::Text,
            clips: vec![Clip::Text(TextClip {
                id: "text-001".into(),
                text: "Title".into(),
                start_ms: 0,
                duration_ms: 1000,
                transform: Transform {
                    x: 0.5,
                    y: 0.1,
                    ..Transform::default()
                },
                color: Some("#ffcc00".into()),
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        let plan = RenderPlan::build(
            &p,
            "#000000",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        );
        let fc = plan.video_filter();
        assert!(fc.contains("textfile='.t/text-001.txt'"), "{fc}");
        assert!(fc.contains("fontcolor=#ffcc00"), "{fc}");
        // must come after the last caption drawtext and before the fade filters
        let last_caption = fc.rfind("caption-002.txt").unwrap();
        let text_pos = fc.find("text-001.txt").unwrap();
        let fade_pos = fc.find("fade=t=in").unwrap();
        assert!(last_caption < text_pos && text_pos < fade_pos, "{fc}");
    }

    #[test]
    fn write_caption_files_also_writes_text_clip_files() {
        let mut p = project(false);
        p.tracks.push(Track {
            id: "text".into(),
            kind: TrackKind::Text,
            clips: vec![Clip::Text(TextClip {
                id: "text-001".into(),
                text: "Hello Title".into(),
                start_ms: 0,
                duration_ms: 1000,
                transform: Transform::default(),
                color: None,
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        let dir = tempfile::tempdir().unwrap();
        let mut plan = RenderPlan::build(
            &p,
            "#000000",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            dir.path().join("scratch"),
            ".t".into(),
            Vec::new(),
        );
        plan.scratch_dir = dir.path().join("scratch");
        plan.write_caption_files().unwrap();
        assert_eq!(
            std::fs::read_to_string(plan.scratch_dir.join("text-001.txt")).unwrap(),
            "Hello Title"
        );
    }

    // -------------------------------------------------------------- audio engine (P1-4)

    fn bgm_clip(start_ms: u64, duration_ms: u64, volume: f32, looping: bool) -> BgmClip {
        BgmClip {
            id: "bgm-001".into(),
            source: RelativeAssetPath::new("assets/bgm/main.mp3").unwrap(),
            start_ms,
            duration_ms,
            volume,
            looping,
            trim_start_ms: 0,
            fade_in_ms: 0,
            fade_out_ms: 0,
            normalize: false,
            extra: BTreeMap::new(),
        }
    }

    fn project_with_bgm(bgm: BgmClip) -> VideoProject {
        let mut p = project(false);
        p.tracks.push(Track {
            id: "bgm".into(),
            kind: TrackKind::Bgm,
            clips: vec![Clip::Bgm(bgm)],
        });
        p
    }

    fn plan_for(p: &VideoProject) -> RenderPlan {
        RenderPlan::build(
            p,
            "#000000",
            SubtitleConfig::default(),
            None,
            "o.mp4".into(),
            "/p/.t".into(),
            ".t".into(),
            Vec::new(),
        )
    }

    #[test]
    fn audio_filter_with_no_audio_at_all_uses_anullsrc() {
        let p = VideoProject::new("s", "S", VideoSettings::default());
        let plan = plan_for(&p);
        assert_eq!(
            plan.audio_filter(),
            format!("anullsrc=r={AUDIO_SAMPLE_RATE}:cl=stereo[aout]")
        );
    }

    #[test]
    fn audio_filter_mixes_dialogue_bgm_and_se() {
        let mut p = project_with_bgm(bgm_clip(0, 7290, 0.6, true));
        p.tracks.push(Track {
            id: "se".into(),
            kind: TrackKind::SoundEffect,
            clips: vec![Clip::SoundEffect(SoundEffectClip {
                id: "se-001".into(),
                source: RelativeAssetPath::new("assets/se/pop.wav").unwrap(),
                start_ms: 1200,
                duration_ms: 800,
                volume: 1.0,
                extra: BTreeMap::new(),
            })],
        });
        let plan = plan_for(&p);
        let fc = plan.audio_filter();
        // 2 dialogue inputs (1,2) + 1 bgm + 1 se = 4 mixed labels
        assert!(
            fc.contains("amix=inputs=4:duration=longest:normalize=0[aout]"),
            "{fc}"
        );
        assert!(fc.contains("[bgm0]"), "{fc}");
        assert!(fc.contains("[se0]"), "{fc}");
    }

    #[test]
    fn audio_filter_ducks_bgm_under_dialogue_windows() {
        let plan = plan_for(&project_with_bgm(bgm_clip(0, 7290, 1.0, false)));
        let fc = plan.audio_filter();
        // project(false) dialogue: (0,3410) and (3610,7290) -> end = start+duration
        assert!(
            fc.contains(&format!(
                "volume=eval=frame:volume='if(between(t,0,3.41)+between(t,3.61,7.29)\\,{:.4}\\,1)'",
                BGM_DUCK_VOLUME
            )),
            "{fc}"
        );
    }

    #[test]
    fn audio_filter_bgm_fade_in_and_out() {
        let mut bgm = bgm_clip(1000, 4000, 1.0, false);
        bgm.fade_in_ms = 500;
        bgm.fade_out_ms = 1000;
        let plan = plan_for(&project_with_bgm(bgm));
        let fc = plan.audio_filter();
        assert!(fc.contains("afade=t=in:st=1:d=0.5"), "{fc}");
        assert!(fc.contains("afade=t=out:st=4:d=1"), "{fc}");
    }

    #[test]
    fn audio_filter_bgm_normalize_applies_dynaudnorm() {
        let mut bgm = bgm_clip(0, 1000, 1.0, false);
        bgm.normalize = true;
        let plan = plan_for(&project_with_bgm(bgm));
        assert!(plan.audio_filter().contains("dynaudnorm"));
    }

    #[test]
    fn audio_filter_looping_bgm_is_trimmed_to_its_own_duration() {
        let plan = plan_for(&project_with_bgm(bgm_clip(2000, 5000, 1.0, true)));
        let fc = plan.audio_filter();
        assert!(fc.contains("atrim=start=0:end=5"), "{fc}");
    }

    #[test]
    fn audio_filter_no_dialogue_means_no_ducking() {
        let p = VideoProject::new("s", "S", VideoSettings::default());
        let plan = plan_for(&project_with_bgm(bgm_clip(0, 1000, 1.0, false)));
        // sanity: this project (built from project_with_bgm) DOES have
        // dialogue; use a bgm-only project instead to check the no-dialogue path.
        let mut bgm_only = p;
        bgm_only.tracks.push(Track {
            id: "bgm".into(),
            kind: TrackKind::Bgm,
            clips: vec![Clip::Bgm(bgm_clip(0, 1000, 1.0, false))],
        });
        let plan_no_dialogue = plan_for(&bgm_only);
        assert!(!plan_no_dialogue.audio_filter().contains("if("));
        // the dialogue-bearing plan, by contrast, does duck.
        assert!(plan.audio_filter().contains("if("));
    }

    #[test]
    fn build_args_appends_bgm_and_se_inputs_after_character_sprites() {
        let plan = plan_for(&project_with_bgm(bgm_clip(0, 1000, 1.0, true)));
        let args: Vec<String> = build_args(&plan)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let bgm_i = args
            .iter()
            .position(|a| a == "assets/bgm/main.mp3")
            .unwrap();
        assert_eq!(args[bgm_i - 3], "-stream_loop");
        let fc = &args[args.iter().position(|x| x == "-filter_complex").unwrap() + 1];
        // project(false) has 2 dialogue inputs -> bgm starts at input 3
        // (0=bg,1,2=dialogue)
        assert!(fc.contains("[3:a]"), "{fc}");
    }
}
