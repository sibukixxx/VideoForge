//! Generation pipeline (design §31, §33, §34).
//!
//! ```text
//! Parse → Validate → TTS → Timeline → Project IR → SRT → Preview → manifest
//! ```
//!
//! Everything is written to `.generated-tmp/<slug>-<nonce>/` and atomically
//! renamed to `generated/<slug>/` on success.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use videoforge_platform::platform_label;
use videoforge_project::{RelativeAssetPath, SourceInfo, VideoProject};
use videoforge_timeline::srt::{self, SrtOptions};
use videoforge_timeline::{DialogueInput, TimelineInput, TimelineOptions};

use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{Manifest, MANIFEST_SCHEMA_VERSION};
use crate::preview::{PreviewRenderer, PreviewRequest};
use crate::progress::{GenerationStage, NoopProgress, ProgressSink};
use crate::tts::{synthesize_all, SynthesisJob, TtsCache, TtsEngine};
use crate::validate;
use crate::workspace::Workspace;
use crate::GENERATOR_VERSION;

pub const PROJECT_FILE: &str = "project.vfp.json";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const CAPTIONS_FILE: &str = "captions.srt";
pub const PREVIEW_FILE: &str = "preview.mp4";
pub const SOURCE_FILE: &str = "source.md";

#[derive(Clone)]
pub struct GenerateOptions {
    /// Override `preview.enabled` from the config.
    pub preview: Option<bool>,
    pub srt_include_speaker: bool,
    pub cancel: CancellationToken,
    /// Keep the temp directory when generation fails (debugging).
    pub keep_tmp_on_failure: bool,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            preview: None,
            srt_include_speaker: false,
            cancel: CancellationToken::new(),
            keep_tmp_on_failure: false,
        }
    }
}

#[derive(Clone)]
pub struct GenerateDeps {
    pub tts: Arc<dyn TtsEngine>,
    pub cache: Option<Arc<TtsCache>>,
    pub preview: Option<Arc<dyn PreviewRenderer>>,
    pub progress: Arc<dyn ProgressSink>,
}

impl GenerateDeps {
    pub fn new(tts: Arc<dyn TtsEngine>) -> Self {
        Self {
            tts,
            cache: None,
            preview: None,
            progress: Arc::new(NoopProgress),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedProject {
    pub slug: String,
    pub output_dir: PathBuf,
    pub project_path: PathBuf,
    pub project: VideoProject,
    pub manifest: Manifest,
    pub preview_path: Option<PathBuf>,
    pub warnings: Vec<String>,
}

pub async fn generate(
    workspace: &Workspace,
    script_path: &Path,
    options: GenerateOptions,
    deps: GenerateDeps,
) -> Result<GeneratedProject, AppError> {
    let progress = Arc::clone(&deps.progress);
    let config = workspace.load_config()?;

    progress.on_stage(&GenerationStage::Parsing);
    let script = videoforge_script::parse_file(script_path)?;

    progress.on_stage(&GenerationStage::Validating);
    let report = validate::validate_script(&script, &config, workspace).into_result()?;
    let mut warnings: Vec<String> = report
        .warnings
        .iter()
        .map(|w| match w.line {
            Some(l) => format!("line {l}: {}", w.message),
            None => w.message.clone(),
        })
        .collect();

    let slug = report.slug.clone();
    let tmp_dir = make_tmp_dir(workspace, &slug)?;
    let result = run_pipeline(
        workspace,
        &config,
        script_path,
        &report,
        &tmp_dir,
        &options,
        &deps,
        &mut warnings,
    )
    .await;

    match result {
        Ok((project, manifest, preview_path)) => {
            let output_dir = workspace.generated_dir().join(&slug);
            promote(&tmp_dir, &output_dir)?;
            progress.on_stage(&GenerationStage::Completed);
            Ok(GeneratedProject {
                slug,
                project_path: output_dir.join(PROJECT_FILE),
                preview_path: preview_path.map(|_| output_dir.join(PREVIEW_FILE)),
                output_dir,
                project,
                manifest,
                warnings,
            })
        }
        Err(e) => {
            if !options.keep_tmp_on_failure {
                let _ = std::fs::remove_dir_all(&tmp_dir);
            }
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_pipeline(
    workspace: &Workspace,
    config: &Config,
    script_path: &Path,
    report: &validate::ValidationReport,
    tmp_dir: &Path,
    options: &GenerateOptions,
    deps: &GenerateDeps,
    warnings: &mut Vec<String>,
) -> Result<(VideoProject, Manifest, Option<PathBuf>), AppError> {
    let progress = &deps.progress;
    let cancel = &options.cancel;

    // --- TTS -------------------------------------------------------------
    let jobs: Vec<SynthesisJob> = report
        .dialogues
        .iter()
        .map(|d| SynthesisJob {
            index: d.index,
            speaker_key: d.speaker_key.clone(),
            speaker_display: d.speaker_display.clone(),
            text: d.text.clone(),
            voice: d.voice,
        })
        .collect();
    let synthesized = synthesize_all(
        Arc::clone(&deps.tts),
        deps.cache.clone(),
        jobs,
        config.tts.concurrency,
        cancel.clone(),
        Arc::clone(progress),
    )
    .await?;

    let audio_dir = tmp_dir.join("assets").join("audio");
    std::fs::create_dir_all(&audio_dir).map_err(|e| AppError::write(&audio_dir, e))?;
    let mut dialogues = Vec::with_capacity(synthesized.len());
    let mut audio_files = Vec::with_capacity(synthesized.len());
    for s in &synthesized {
        let file = format!("{:03}.wav", s.index);
        let path = audio_dir.join(&file);
        std::fs::write(&path, &s.wav).map_err(|e| AppError::write(&path, e))?;
        let rel = RelativeAssetPath::new(format!("assets/audio/{file}"))?;
        audio_files.push(rel.as_str().to_string());
        dialogues.push(DialogueInput {
            index: s.index,
            speaker: s.speaker_key.clone(),
            speaker_display: s.speaker_display.clone(),
            text: s.text.clone(),
            audio: rel,
            duration_ms: s.duration_ms,
        });
    }

    // --- Background asset ------------------------------------------------
    let background = match &config.preview.background {
        Some(bg) => {
            let src = workspace.resolve(bg)?;
            if src.is_file() {
                let name = src
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "background".into());
                let dst_dir = tmp_dir.join("assets").join("background");
                std::fs::create_dir_all(&dst_dir).map_err(|e| AppError::write(&dst_dir, e))?;
                let dst = dst_dir.join(&name);
                std::fs::copy(&src, &dst).map_err(|e| AppError::write(&dst, e))?;
                Some(RelativeAssetPath::new(format!("assets/background/{name}"))?)
            } else {
                None
            }
        }
        None => None,
    };

    // --- Timeline --------------------------------------------------------
    if cancel.is_cancelled() {
        return Err(AppError::Cancelled);
    }
    progress.on_stage(&GenerationStage::BuildingTimeline);
    let source_rel = workspace.relative(script_path);
    let project = videoforge_timeline::build(TimelineInput {
        id: report.slug.clone(),
        title: report.title.clone(),
        video: config.video.into(),
        source: SourceInfo {
            script: Some(source_rel.clone()),
            template: report.template.clone(),
            generator_version: Some(GENERATOR_VERSION.to_string()),
        },
        dialogues,
        background,
        options: TimelineOptions {
            dialogue_gap_ms: config.timeline.dialogue_gap_ms,
        },
    })?;

    // --- Project IR + captions + source copy -----------------------------
    progress.on_stage(&GenerationStage::WritingProject);
    let project_path = tmp_dir.join(PROJECT_FILE);
    project.save(&project_path)?;
    let source_copy = tmp_dir.join(SOURCE_FILE);
    std::fs::copy(script_path, &source_copy).map_err(|e| AppError::write(&source_copy, e))?;

    progress.on_stage(&GenerationStage::WritingCaptions);
    let captions_path = tmp_dir.join(CAPTIONS_FILE);
    let srt_text = srt::render(
        &project,
        SrtOptions {
            include_speaker: options.srt_include_speaker,
        },
    );
    std::fs::write(&captions_path, srt_text).map_err(|e| AppError::write(&captions_path, e))?;

    // --- Preview ---------------------------------------------------------
    let preview_enabled = options.preview.unwrap_or(config.preview.enabled);
    let mut preview_path = None;
    if preview_enabled {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        match &deps.preview {
            Some(renderer) => match renderer.availability() {
                Ok(_) => {
                    progress.on_stage(&GenerationStage::RenderingPreview);
                    let output = tmp_dir.join(PREVIEW_FILE);
                    let font = config
                        .preview
                        .font
                        .as_deref()
                        .and_then(|f| workspace.resolve_allow_absolute(f).ok())
                        .filter(|p| p.is_file());
                    renderer
                        .render(PreviewRequest {
                            project: &project,
                            project_dir: tmp_dir,
                            output: &output,
                            font: font.as_deref(),
                            background_color: &config.preview.background_color,
                            cancel: cancel.clone(),
                        })
                        .await?;
                    preview_path = Some(output);
                }
                Err(e) => {
                    let reason = format!("preview skipped: {e}");
                    progress.on_stage(&GenerationStage::PreviewSkipped {
                        reason: reason.clone(),
                    });
                    warnings.push(reason);
                }
            },
            None => {
                let reason = "preview skipped: no renderer configured".to_string();
                progress.on_stage(&GenerationStage::PreviewSkipped {
                    reason: reason.clone(),
                });
                warnings.push(reason);
            }
        }
    }

    // --- Manifest --------------------------------------------------------
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        generator_version: GENERATOR_VERSION.to_string(),
        source: source_rel,
        project: PROJECT_FILE.to_string(),
        preview: preview_path.as_ref().map(|_| PREVIEW_FILE.to_string()),
        captions: CAPTIONS_FILE.to_string(),
        audio: audio_files,
        generated_at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        platform: platform_label(),
        duration_ms: project.total_duration_ms(),
        dialogues: project.audio_clips().len(),
        warnings: warnings.clone(),
    };
    manifest.save(&tmp_dir.join(MANIFEST_FILE))?;

    Ok((project, manifest, preview_path))
}

fn make_tmp_dir(workspace: &Workspace, slug: &str) -> Result<PathBuf, AppError> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = workspace
        .tmp_dir()
        .join(format!("{slug}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| AppError::write(&dir, e))?;
    Ok(dir)
}

/// Replace `output_dir` with `tmp_dir` as atomically as the filesystem allows.
fn promote(tmp_dir: &Path, output_dir: &Path) -> Result<(), AppError> {
    if let Some(parent) = output_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
    }
    if output_dir.exists() {
        let old = output_dir.with_extension("old");
        if old.exists() {
            std::fs::remove_dir_all(&old).map_err(|e| AppError::write(&old, e))?;
        }
        std::fs::rename(output_dir, &old).map_err(|e| AppError::write(output_dir, e))?;
        let moved = std::fs::rename(tmp_dir, output_dir);
        if let Err(e) = moved {
            // roll back
            let _ = std::fs::rename(&old, output_dir);
            return Err(AppError::write(output_dir, e));
        }
        let _ = std::fs::remove_dir_all(&old);
    } else {
        std::fs::rename(tmp_dir, output_dir).map_err(|e| AppError::write(output_dir, e))?;
    }
    // best-effort cleanup of the temp parent if empty
    if let Some(parent) = tmp_dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use crate::tts::FakeTtsEngine;

    #[tokio::test]
    async fn end_to_end_with_fake_tts() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let script = ws.scripts_dir().join("sample.md");

        let deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
        let out = generate(&ws, &script, GenerateOptions::default(), deps)
            .await
            .unwrap();

        assert_eq!(out.slug, "sample");
        assert_eq!(out.output_dir, ws.generated_dir().join("sample"));
        for f in [PROJECT_FILE, MANIFEST_FILE, CAPTIONS_FILE, SOURCE_FILE] {
            assert!(out.output_dir.join(f).is_file(), "{f}");
        }
        assert!(out.output_dir.join("assets/audio/001.wav").is_file());
        assert!(out.output_dir.join("assets/audio/003.wav").is_file());
        assert!(!ws.tmp_dir().exists(), "temp dir cleaned up");

        let project = VideoProject::load(&out.project_path).unwrap();
        assert_eq!(project.audio_clips().len(), 3);
        assert_eq!(project.source.script.as_deref(), Some("scripts/sample.md"));
        let a = project.audio_clips();
        assert_eq!(a[1].start_ms, a[0].duration_ms + 200);
        assert!(out.manifest.preview.is_none());
        assert!(out.warnings.iter().any(|w| w.contains("preview skipped")));
        assert_eq!(out.manifest.dialogues, 3);

        let srt = std::fs::read_to_string(out.output_dir.join(CAPTIONS_FILE)).unwrap();
        assert!(srt.starts_with("1\n00:00:00,000 --> "));

        // regenerate replaces the directory
        let deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
        let again = generate(&ws, &script, GenerateOptions::default(), deps)
            .await
            .unwrap();
        assert_eq!(again.output_dir, out.output_dir);
        assert!(!out.output_dir.with_extension("old").exists());
    }

    #[tokio::test]
    async fn invalid_script_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let script = ws.scripts_dir().join("bad.md");
        std::fs::write(&script, "だれか:\nやあ\n").unwrap();
        let deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
        let err = generate(&ws, &script, GenerateOptions::default(), deps)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidScript(_)), "{err}");
        assert!(!ws.generated_dir().join("bad").exists());
    }
}
