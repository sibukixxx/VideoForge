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

use videoforge_core::lipsync::{mouth_segments, LipSyncTrack, MouthState};
use videoforge_core::preview::{CharacterSpriteSet, PreviewRequest};
use videoforge_core::project::{Transform, VideoProject};
use videoforge_core::AppError;

pub const AUDIO_SAMPLE_RATE: u32 = 48000;
pub const FADE_SECS: f64 = 0.3;
/// A `png_lipsync` character sprite is scaled to this fraction of the frame
/// height at `presentation.scale == 1.0` (P0-1). Hard-coded but named, same
/// reasoning as the lip-sync amplitude thresholds in `core::lipsync`.
pub const CHARACTER_BASE_HEIGHT_FRACTION: f64 = 0.62;

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

#[derive(Debug, Clone, PartialEq)]
pub struct CaptionPlan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
    pub text: String,
    /// Relative to the project dir.
    pub text_file: String,
    pub speaker_file: String,
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
    /// Relative audio paths and their start offsets.
    pub audio: Vec<(String, u64)>,
    pub captions: Vec<CaptionPlan>,
    pub font: Option<PathBuf>,
    pub output: PathBuf,
    /// Absolute scratch directory (caption text files are written here).
    pub scratch_dir: PathBuf,
    /// `scratch_dir` relative to the project dir.
    pub scratch_rel: String,
    /// `png_lipsync` character overlays (P0-1). Empty for a project that
    /// uses no character, or only Live2D ones.
    pub character_overlays: Vec<CharacterOverlayPlan>,
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
            .map(|a| (a.source.as_str().to_string(), a.start_ms))
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
            font,
            output,
            scratch_dir,
            scratch_rel,
            character_overlays,
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
        Ok(())
    }

    fn caption_font_size(&self) -> u32 {
        (self.height as f64 * 0.048).round() as u32
    }

    fn speaker_font_size(&self) -> u32 {
        (self.height as f64 * 0.034).round() as u32
    }

    fn max_chars_per_line(&self) -> usize {
        // CJK glyphs are ~1em wide; leave 10% margins.
        let usable = self.width as f64 * 0.8;
        (usable / self.caption_font_size() as f64).floor().max(8.0) as usize
    }

    /// Pixel height of the bottom "caption safe area" (speaker label +
    /// caption text + their margins), measured from the bottom edge. Shared
    /// by the caption `drawtext` y-position and the character overlay
    /// clamp below, so a character can never be placed under the captions —
    /// structural, not incidental (P0-1: "字幕との重なりを考慮できる構造").
    fn caption_safe_area_px(&self) -> u32 {
        (self.height as f64 * 0.20).round() as u32 + self.speaker_font_size() + 16
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
    /// clamped to stay fully inside the frame and above the caption safe
    /// area. `overlay_w`/`overlay_h` are resolved by FFmpeg at run time from
    /// the actual scaled sprite, so this does not need to know pixel sizes.
    fn overlay_position_exprs(&self, transform: &Transform) -> (String, String) {
        let safe_area = self.caption_safe_area_px();
        let x = format!(
            "min(max(0,{:.4}*main_w-overlay_w/2),main_w-overlay_w)",
            transform.x
        );
        let y = format!(
            "min(max(0,{:.4}*main_h-overlay_h/2),main_h-{safe_area}-overlay_h)",
            transform.y
        );
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
            let base_input = 1 + self.audio.len() + i * 3;
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
        let caption_y = format!("h-{}", (h as f64 * 0.20).round() as u32);
        let speaker_y = format!("h-{}", (h as f64 * 0.20).round() as u32 + speaker_size + 16);
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
                "drawtext=textfile={}{font}:fontsize={caption_size}:fontcolor=white:borderw=3:bordercolor=black:line_spacing=8:text_align=center:x=(w-text_w)/2:y={caption_y}:{enable}",
                quote_filter_value(&c.text_file)
            ));
        }
        let total = self.total_ms as f64 / 1000.0;
        tail.push(format!("fade=t=in:st=0:d={FADE_SECS}"));
        tail.push(format!(
            "fade=t=out:st={}:d={FADE_SECS}",
            fmt_secs((total - FADE_SECS).max(0.0))
        ));

        if self.character_overlays.is_empty() {
            let chain: Vec<String> = base.into_iter().chain(tail).collect();
            return format!("[0:v]{}[v]", chain.join(","));
        }

        let mut parts = vec![format!("[0:v]{}[bg0]", base.join(","))];
        let (overlay_filters, post_overlay_label) = self.character_filters("bg0");
        parts.extend(overlay_filters);
        parts.push(format!("[{post_overlay_label}]{}[v]", tail.join(",")));
        parts.join(";")
    }

    pub fn audio_filter(&self) -> String {
        if self.audio.is_empty() {
            return format!("anullsrc=r={AUDIO_SAMPLE_RATE}:cl=stereo[aout]");
        }
        let mut parts = Vec::new();
        let mut labels = Vec::new();
        for (i, (_, start)) in self.audio.iter().enumerate() {
            let label = format!("[a{}]", i + 1);
            parts.push(format!(
                "[{}:a]aresample={AUDIO_SAMPLE_RATE},aformat=channel_layouts=stereo,adelay={start}:all=1{label}",
                i + 1
            ));
            labels.push(label);
        }
        if self.audio.len() == 1 {
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
    for (path, _) in &plan.audio {
        args.push("-i".into());
        args.push(path.into());
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
        AudioClip, BackgroundClip, CaptionClip, Clip, RelativeAssetPath, Track, TrackKind,
        VideoSettings,
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
                    extra: BTreeMap::new(),
                }),
                Clip::Caption(CaptionClip {
                    id: "c2".into(),
                    text: "やあ".into(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    speaker_display: None,
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
}
