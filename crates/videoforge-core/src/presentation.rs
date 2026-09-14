//! Marp Markdown adapter for issue #59.
//!
//! Marp remains an external executable. VideoForge validates trusted,
//! workspace-local Markdown, materializes PNG files, and then forgets about
//! Marp: the canonical project contains ordinary `Clip::Image` values only.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use videoforge_project::{
    Clip, FitMode, ImageClip, Presentation, RelativeAssetPath, Track, TrackKind, Transform,
    VideoProject,
};

use crate::{AppError, Workspace};

pub const THEME_FILE: &str = "videoforge.css";
pub const SOURCE_FILE: &str = "presentation.md";
pub const MAX_SOURCE_BYTES: u64 = 1_000_000;
pub const MAX_SLIDE_CHARS: usize = 360;
pub const THEME: &str = include_str!("../../../assets/marp/videoforge.css");
pub const PROMPT: &str = include_str!("../../../prompts/presentation-marp-p0.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Warning,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationDiagnostic {
    pub code: String,
    pub level: DiagnosticLevel,
    pub path: Option<String>,
    pub detail: String,
    pub remediation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationValidationReport {
    pub source: String,
    pub slide_count: usize,
    pub diagnostics: Vec<PresentationDiagnostic>,
}

impl PresentationValidationReport {
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|item| item.level == DiagnosticLevel::Failure)
    }

    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|item| item.level == DiagnosticLevel::Warning)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarpInfo {
    pub executable: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarpCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub current_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedPresentation {
    pub output_dir: PathBuf,
    pub slides: Vec<PathBuf>,
    pub renderer: String,
}

pub trait PresentationRenderer: Send + Sync {
    fn id(&self) -> &'static str;
    fn availability(&self) -> Result<MarpInfo, AppError>;
    fn render(&self, source: &Path, output_dir: &Path) -> Result<RenderedPresentation, AppError>;
}

#[derive(Debug, Clone)]
pub struct MarpCli {
    executable: PathBuf,
}

impl MarpCli {
    pub fn detect() -> Self {
        Self {
            executable: std::env::var_os("VIDEOFORGE_MARP")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("marp")),
        }
    }

    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn command(&self, source: &Path, output_dir: &Path) -> MarpCommand {
        let theme = output_dir.join(THEME_FILE);
        let output = output_dir.join("slide.png");
        MarpCommand {
            program: self.executable.clone(),
            args: vec![
                OsString::from("--images"),
                OsString::from("png"),
                OsString::from("--image-scale"),
                OsString::from("1.5"),
                OsString::from("--allow-local-files"),
                OsString::from("--theme-set"),
                theme.into_os_string(),
                OsString::from("--output"),
                output.into_os_string(),
                source.as_os_str().to_os_string(),
            ],
            current_dir: source
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
        }
    }
}

impl Default for MarpCli {
    fn default() -> Self {
        Self::detect()
    }
}

impl PresentationRenderer for MarpCli {
    fn id(&self) -> &'static str {
        "marp"
    }

    fn availability(&self) -> Result<MarpInfo, AppError> {
        let output = Command::new(&self.executable)
            .arg("--version")
            .output()
            .map_err(|error| {
                AppError::MarpUnavailable(format!(
                    "`{}` could not be executed ({error}); install Marp CLI or set VIDEOFORGE_MARP",
                    self.executable.display()
                ))
            })?;
        if !output.status.success() {
            return Err(AppError::MarpUnavailable(format!(
                "`{} --version` exited with {}",
                self.executable.display(),
                output.status
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let version = stdout
            .lines()
            .chain(stderr.lines())
            .find(|line| !line.trim().is_empty())
            .unwrap_or("marp")
            .trim()
            .to_string();
        Ok(MarpInfo {
            executable: self.executable.display().to_string(),
            version,
        })
    }

    fn render(&self, source: &Path, output_dir: &Path) -> Result<RenderedPresentation, AppError> {
        self.availability()?;
        std::fs::create_dir_all(output_dir).map_err(|e| AppError::write(output_dir, e))?;
        std::fs::write(output_dir.join(THEME_FILE), THEME)
            .map_err(|e| AppError::write(output_dir.join(THEME_FILE), e))?;

        let command = self.command(source, output_dir);
        let output = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.current_dir)
            .output()
            .map_err(|error| {
                AppError::PresentationRenderFailed(format!(
                    "failed to start `{}`: {error}",
                    command.program.display()
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::PresentationRenderFailed(format!(
                "Marp exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        let slides = normalize_slide_names(output_dir)?;
        if slides.is_empty() {
            return Err(AppError::PresentationRenderFailed(
                "Marp exited successfully but wrote no slide PNG files".into(),
            ));
        }
        std::fs::copy(source, output_dir.join(SOURCE_FILE))
            .map_err(|e| AppError::write(output_dir.join(SOURCE_FILE), e))?;
        Ok(RenderedPresentation {
            output_dir: output_dir.to_path_buf(),
            slides,
            renderer: self.id().into(),
        })
    }
}

pub fn prompt(workspace: &Workspace, script: &Path) -> Result<String, AppError> {
    let metadata = std::fs::metadata(script).map_err(|e| AppError::read(script, e))?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::InvalidPresentation(
            "script input exceeds 1 MB".into(),
        ));
    }
    let source = std::fs::read_to_string(script).map_err(|e| AppError::read(script, e))?;
    Ok(format!(
        "{PROMPT}\n\n## INPUT_SCRIPT\n\nPath: {}\n\n```markdown\n{}\n```\n",
        workspace.relative(script),
        source.trim_end()
    ))
}

pub fn validate(
    workspace: &Workspace,
    source: &Path,
) -> Result<PresentationValidationReport, AppError> {
    let metadata = std::fs::metadata(source).map_err(|e| AppError::read(source, e))?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::InvalidPresentation(
            "presentation source exceeds 1 MB".into(),
        ));
    }
    let markdown = std::fs::read_to_string(source)
        .map_err(|e| AppError::read(source, e))?
        .replace("\r\n", "\n");
    let mut diagnostics = Vec::new();
    let (front_matter, body) = match split_front_matter(&markdown) {
        Some(parts) => parts,
        None => {
            diagnostics.push(diagnostic(
                "marp_front_matter",
                DiagnosticLevel::Failure,
                Some(workspace.relative(source)),
                "missing YAML front matter",
                "add front matter with `marp: true`, `theme: videoforge`, and `size: 16:9`",
            ));
            ("", markdown.as_str())
        }
    };

    if !front_matter.is_empty() {
        match serde_yaml::from_str::<serde_yaml::Value>(front_matter) {
            Ok(value) => {
                require_front_matter(&value, "marp", "true", &mut diagnostics);
                require_front_matter(&value, "theme", "videoforge", &mut diagnostics);
                require_front_matter(&value, "size", "16:9", &mut diagnostics);
            }
            Err(error) => diagnostics.push(diagnostic(
                "marp_front_matter",
                DiagnosticLevel::Failure,
                Some(workspace.relative(source)),
                format!("invalid YAML front matter: {error}"),
                "repair the YAML front matter before rendering",
            )),
        }
    }

    let slides = split_slides(body);
    if slides.is_empty() {
        diagnostics.push(diagnostic(
            "slide_count",
            DiagnosticLevel::Failure,
            Some(workspace.relative(source)),
            "presentation has no slides",
            "add at least one non-empty slide",
        ));
    }
    for (index, slide) in slides.iter().enumerate() {
        let chars = visible_chars(slide);
        if chars == 0 {
            diagnostics.push(diagnostic(
                "empty_slide",
                DiagnosticLevel::Failure,
                Some(format!("slide:{}", index + 1)),
                "slide is empty",
                "remove the separator or add one message to the slide",
            ));
        }
        if chars > MAX_SLIDE_CHARS {
            diagnostics.push(diagnostic(
                "slide_too_dense",
                DiagnosticLevel::Warning,
                Some(format!("slide:{}", index + 1)),
                format!("slide has {chars} visible characters (recommended <= {MAX_SLIDE_CHARS})"),
                "shorten the slide; keep narration detail in the script",
            ));
        }
        for image in markdown_images(slide) {
            validate_image_path(workspace, source, &image, index + 1, &mut diagnostics);
        }
    }

    Ok(PresentationValidationReport {
        source: workspace.relative(source),
        slide_count: slides.len(),
        diagnostics,
    })
}

pub fn attach_slides(
    project: &mut VideoProject,
    slide_sources: &[RelativeAssetPath],
) -> Result<(), AppError> {
    let mut audio: Vec<_> = project.audio_clips();
    audio.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then_with(|| a.id.cmp(&b.id)));
    if audio.len() != slide_sources.len() {
        return Err(AppError::PresentationSlideMismatch {
            slides: slide_sources.len(),
            narration_segments: audio.len(),
        });
    }
    if project.tracks.iter().any(|track| track.id == "presentation") {
        return Err(AppError::InvalidPresentation(
            "project already has a track named `presentation`".into(),
        ));
    }
    let clips = audio
        .iter()
        .zip(slide_sources)
        .enumerate()
        .map(|(index, (segment, source))| {
            Clip::Image(ImageClip {
                id: format!("presentation-{:03}", index + 1),
                source: source.clone(),
                start_ms: segment.start_ms,
                duration_ms: segment.duration_ms,
                transform: Transform {
                    fit: FitMode::Cover,
                    layer: -100,
                    ..Transform::default()
                },
                presentation: Some(Presentation {
                    role: Some("primary_visual".into()),
                    intent: Some("cut".into()),
                    intent_duration_ms: None,
                }),
                extra: BTreeMap::new(),
            })
        })
        .collect();
    project.tracks.push(Track {
        id: "presentation".into(),
        kind: TrackKind::Image,
        clips,
    });
    Ok(())
}

pub fn slide_asset_paths(count: usize) -> Result<Vec<RelativeAssetPath>, AppError> {
    (1..=count)
        .map(|index| RelativeAssetPath::new(format!("presentation/slide-{index:03}.png")))
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)
}

fn diagnostic(
    code: &str,
    level: DiagnosticLevel,
    path: Option<String>,
    detail: impl Into<String>,
    remediation: &str,
) -> PresentationDiagnostic {
    PresentationDiagnostic {
        code: code.into(),
        level,
        path,
        detail: detail.into(),
        remediation: remediation.into(),
    }
}

fn split_front_matter(markdown: &str) -> Option<(&str, &str)> {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let rest = markdown.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    Some((&rest[..end], &rest[end + 5..]))
}

fn require_front_matter(
    value: &serde_yaml::Value,
    key: &str,
    expected: &str,
    diagnostics: &mut Vec<PresentationDiagnostic>,
) {
    let actual = value
        .as_mapping()
        .and_then(|map| map.get(&serde_yaml::Value::String(key.into())))
        .map(|value| match value {
            serde_yaml::Value::Bool(value) => value.to_string(),
            serde_yaml::Value::String(value) => value.clone(),
            _ => String::new(),
        });
    if actual.as_deref() != Some(expected) {
        diagnostics.push(diagnostic(
            "marp_front_matter",
            DiagnosticLevel::Failure,
            Some(key.into()),
            format!("expected `{key}: {expected}`"),
            "use the VideoForge Marp front matter contract",
        ));
    }
}

fn split_slides(body: &str) -> Vec<String> {
    let mut slides = Vec::new();
    let mut current = Vec::new();
    let mut fence: Option<&str> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fence = if fence == Some("```") {
                None
            } else if fence.is_none() {
                Some("```")
            } else {
                fence
            };
        } else if trimmed.starts_with("~~~") {
            fence = if fence == Some("~~~") {
                None
            } else if fence.is_none() {
                Some("~~~")
            } else {
                fence
            };
        }
        if fence.is_none() && line.trim() == "---" {
            slides.push(current.join("\n"));
            current.clear();
        } else {
            current.push(line);
        }
    }
    slides.push(current.join("\n"));
    if slides.len() == 1 && slides[0].trim().is_empty() {
        Vec::new()
    } else {
        slides
    }
}

fn visible_chars(slide: &str) -> usize {
    slide
        .lines()
        .filter(|line| !line.trim_start().starts_with("<!--"))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace() && !"#*_`|>-".contains(*c))
        .count()
}

fn markdown_images(markdown: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut remaining = markdown;
    while let Some(start) = remaining.find("![") {
        remaining = &remaining[start + 2..];
        let Some(close_alt) = remaining.find("](") else {
            break;
        };
        remaining = &remaining[close_alt + 2..];
        let Some(close_path) = remaining.find(')') else {
            break;
        };
        let raw = remaining[..close_path].trim();
        let path = raw
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .unwrap_or(raw)
            .split_whitespace()
            .next()
            .unwrap_or("");
        if !path.is_empty() {
            result.push(path.to_string());
        }
        remaining = &remaining[close_path + 1..];
    }
    result
}

fn validate_image_path(
    workspace: &Workspace,
    source: &Path,
    image: &str,
    slide: usize,
    diagnostics: &mut Vec<PresentationDiagnostic>,
) {
    if image.starts_with("http://")
        || image.starts_with("https://")
        || image.starts_with("data:")
    {
        diagnostics.push(diagnostic(
            "remote_presentation_asset",
            DiagnosticLevel::Failure,
            Some(format!("slide:{slide}")),
            format!("non-local image source is not allowed: {image}"),
            "download and review the image, then reference a workspace-relative asset path",
        ));
        return;
    }
    let source_dir = source.parent().unwrap_or_else(|| Path::new("."));
    let base = source_dir.strip_prefix(workspace.root()).unwrap_or(source_dir);
    let relative = base.join(image).to_string_lossy().into_owned();
    match workspace.resolve(&relative) {
        Ok(path) if path.is_file() => {}
        Ok(_) => diagnostics.push(diagnostic(
            "missing_presentation_asset",
            DiagnosticLevel::Failure,
            Some(image.into()),
            "referenced image does not exist",
            "restore the image or correct the workspace-relative path",
        )),
        Err(error) => diagnostics.push(diagnostic(
            "unsafe_presentation_path",
            DiagnosticLevel::Failure,
            Some(image.into()),
            error.to_string(),
            "use a workspace-relative path without `..`, an absolute prefix, or a symlink escape",
        )),
    }
}

fn normalize_slide_names(output_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut numbered = Vec::new();
    for entry in std::fs::read_dir(output_dir).map_err(|e| AppError::read(output_dir, e))? {
        let entry = entry.map_err(|e| AppError::read(output_dir, e))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let index = if name == "slide.png" {
            Some(1)
        } else {
            name.strip_prefix("slide.")
                .and_then(|value| value.strip_suffix(".png"))
                .and_then(|value| value.parse::<usize>().ok())
        };
        if let Some(index) = index {
            numbered.push((index, path));
        }
    }
    numbered.sort_by_key(|(index, _)| *index);
    let mut slides = Vec::with_capacity(numbered.len());
    for (expected, (actual, source)) in numbered.into_iter().enumerate() {
        let expected = expected + 1;
        if actual != expected {
            return Err(AppError::PresentationRenderFailed(format!(
                "Marp slide sequence is not contiguous: expected {expected}, found {actual}"
            )));
        }
        let target = output_dir.join(format!("slide-{expected:03}.png"));
        std::fs::rename(&source, &target).map_err(|e| AppError::write(&target, e))?;
        slides.push(target);
    }
    Ok(slides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use videoforge_project::{AudioClip, SourceInfo, VideoSettings};

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("videoforge.yaml"), "speakers: {}\n").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn command_builder_is_explicit_and_never_uses_npx() {
        let marp = MarpCli::new("/tools/marp");
        let command = marp.command(Path::new("/work/presentation.md"), Path::new("/out"));
        assert_eq!(command.program, Path::new("/tools/marp"));
        let args: Vec<_> = command
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--images",
                "png",
                "--image-scale",
                "1.5",
                "--allow-local-files",
                "--theme-set",
                "/out/videoforge.css",
                "--output",
                "/out/slide.png",
                "/work/presentation.md",
            ]
        );
    }

    #[test]
    fn validation_rejects_invalid_front_matter_empty_slide_and_unsafe_path() {
        let (_dir, ws) = workspace();
        let source = ws.root().join("bad.md");
        std::fs::write(
            &source,
            "---\nmarp: false\ntheme: default\n---\n# one\n---\n\n---\n![](../secret.png)\n",
        )
        .unwrap();
        let report = validate(&ws, &source).unwrap();
        assert!(!report.is_valid());
        assert_eq!(report.slide_count, 3);
        assert!(report.diagnostics.iter().any(|item| item.code == "empty_slide"));
        assert!(report
            .diagnostics
            .iter()
            .any(|item| item.code == "unsafe_presentation_path"));
    }

    #[test]
    fn normalizes_marp_slide_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["slide.003.png", "slide.001.png", "slide.002.png"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let slides = normalize_slide_names(dir.path()).unwrap();
        assert_eq!(
            slides
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["slide-001.png", "slide-002.png", "slide-003.png"]
        );
    }

    #[test]
    fn slide_mapping_uses_existing_audio_timing_and_image_clip() {
        let mut project = VideoProject::new("x", "X", VideoSettings::default());
        project.source = SourceInfo::default();
        project.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![
                Clip::Audio(AudioClip {
                    id: "audio-002".into(),
                    source: RelativeAssetPath::new("assets/audio/002.wav").unwrap(),
                    start_ms: 1200,
                    duration_ms: 800,
                    speaker: "b".into(),
                    extra: BTreeMap::new(),
                }),
                Clip::Audio(AudioClip {
                    id: "audio-001".into(),
                    source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                    start_ms: 0,
                    duration_ms: 1000,
                    speaker: "a".into(),
                    extra: BTreeMap::new(),
                }),
            ],
        });
        attach_slides(&mut project, &slide_asset_paths(2).unwrap()).unwrap();
        let track = project
            .tracks
            .iter()
            .find(|track| track.id == "presentation")
            .unwrap();
        let Clip::Image(first) = &track.clips[0] else {
            panic!("image")
        };
        assert_eq!((first.start_ms, first.duration_ms), (0, 1000));
        assert_eq!(first.source.as_str(), "presentation/slide-001.png");
        assert_eq!(first.transform.layer, -100);
    }

    #[test]
    fn slide_mapping_never_silently_corrects_a_count_mismatch() {
        let mut project = VideoProject::new("x", "X", VideoSettings::default());
        let error = attach_slides(&mut project, &slide_asset_paths(1).unwrap()).unwrap_err();
        assert!(matches!(
            error,
            AppError::PresentationSlideMismatch {
                slides: 1,
                narration_segments: 0
            }
        ));
    }

    #[test]
    fn prompt_is_deterministic_and_contains_the_script() {
        let (_dir, ws) = workspace();
        let script = ws.root().join("script.md");
        std::fs::write(&script, "話者:\n根拠のある台本。\n").unwrap();
        let first = prompt(&ws, &script).unwrap();
        let second = prompt(&ws, &script).unwrap();
        assert_eq!(first, second);
        assert!(first.contains("Marp Markdownだけ"));
        assert!(first.contains("根拠のある台本"));
    }

    #[test]
    fn unavailable_marp_has_a_stable_typed_error() {
        let error = MarpCli::new("/definitely/not/marp")
            .availability()
            .unwrap_err();
        assert!(matches!(error, AppError::MarpUnavailable(_)));
        assert_eq!(error.code(), "marp_unavailable");
    }
}
