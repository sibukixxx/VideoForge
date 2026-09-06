//! Pure FFmpeg command builder (VF-051). No process is spawned here, so the
//! whole plan is unit-testable without FFmpeg installed.
//!
//! Paths inside the filter graph are kept *relative to the project directory*
//! (FFmpeg runs with `cwd = project_dir`), so a workspace path containing
//! spaces, Japanese, quotes or a drive letter never enters the graph at all.
//! Only the optional font file may be absolute; it goes through
//! [`quote_filter_value`], whose escaping rules are documented there and
//! verified against a real FFmpeg in `tests/ffmpeg_real.rs`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use videoforge_core::preview::PreviewRequest;
use videoforge_core::project::VideoProject;
use videoforge_core::AppError;

pub const AUDIO_SAMPLE_RATE: u32 = 48000;
pub const FADE_SECS: f64 = 0.3;

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
        Ok(Self::build(
            request.project,
            request.background_color,
            request.font.map(Path::to_path_buf),
            request.output.to_path_buf(),
            scratch_dir.to_path_buf(),
            scratch_rel,
        ))
    }

    pub fn build(
        project: &VideoProject,
        background_color: &str,
        font: Option<PathBuf>,
        output: PathBuf,
        scratch_dir: PathBuf,
        scratch_rel: String,
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

    pub fn video_filter(&self) -> String {
        let (w, h) = (self.width, self.height);
        let mut chain = vec![
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
        for c in &self.captions {
            let enable = format!(
                "enable='between(t,{},{})'",
                ms_to_secs(c.start_ms),
                ms_to_secs(c.end_ms)
            );
            chain.push(format!(
                "drawtext=textfile={}{font}:fontsize={speaker_size}:fontcolor=white:box=1:boxcolor=0x000000AA:boxborderw=10:x=(w-text_w)/2:y={speaker_y}:{enable}",
                quote_filter_value(&c.speaker_file)
            ));
            chain.push(format!(
                "drawtext=textfile={}{font}:fontsize={caption_size}:fontcolor=white:borderw=3:bordercolor=black:line_spacing=8:text_align=center:x=(w-text_w)/2:y={caption_y}:{enable}",
                quote_filter_value(&c.text_file)
            ));
        }
        let total = self.total_ms as f64 / 1000.0;
        chain.push(format!("fade=t=in:st=0:d={FADE_SECS}"));
        chain.push(format!(
            "fade=t=out:st={}:d={FADE_SECS}",
            fmt_secs((total - FADE_SECS).max(0.0))
        ));
        format!("[0:v]{}[v]", chain.join(","))
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
}
