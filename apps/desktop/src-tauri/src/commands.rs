//! Tauri commands (design §26). Thin wrappers over `videoforge-core`; the
//! only decisions taken here are which concrete engine / renderer / exporter
//! to hand to core — the same choices `videoforge-cli` makes.
//!
//! Every command takes the workspace root explicitly (the GUI may switch
//! workspaces) and returns [`CommandError`] with an `AppError::code()`.
//! Paths coming from the frontend are validated with `Workspace::resolve`
//! or checked to live under `generated/` before they are touched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use videoforge_core::doctor::{self, DoctorInput, DoctorReport};
use videoforge_core::export::{ExportRequest, ProjectExporter};
use videoforge_core::generate::{MANIFEST_FILE, PREVIEW_FILE, PROJECT_FILE};
use videoforge_core::manifest::Manifest;
use videoforge_core::preview::PreviewRenderer;
use videoforge_core::progress::{GenerationStage, ProgressSink};
use videoforge_core::project::VideoProject;
use videoforge_core::tts::{FakeTtsEngine, TtsCache, TtsEngine};
use videoforge_core::validate::ValidationReport;
use videoforge_core::{generate as core_generate, init as core_init, validate as core_validate};
use videoforge_core::{AppError, GenerateDeps, GenerateOptions, Workspace};
use videoforge_export_ymm4::bundle::{create_bundle, BundleOptions};
use videoforge_export_ymm4::Ymm4Exporter;
use videoforge_platform::{current_platform, platform_label, PlatformKind};
use videoforge_preview::FfmpegPreviewRenderer;
use videoforge_voicevox::VoicevoxEngine;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Event name for [`GenerationStage`] payloads during `generate`.
pub const PROGRESS_EVENT: &str = "generate:progress";

// ---------------------------------------------------------------- wiring

fn make_tts(
    fake: bool,
    endpoint: &str,
    timeout_secs: u64,
    allow_remote_endpoint: bool,
) -> CommandResult<Arc<dyn TtsEngine>> {
    if fake {
        return Ok(Arc::new(FakeTtsEngine::default()));
    }
    Ok(Arc::new(VoicevoxEngine::new(
        endpoint,
        Duration::from_secs(timeout_secs.max(1)),
        allow_remote_endpoint,
    )?))
}

fn make_cache() -> Option<Arc<TtsCache>> {
    current_platform()
        .cache_dir()
        .ok()
        .map(|dir| Arc::new(TtsCache::in_platform_cache(&dir)))
}

fn open(root: &str) -> CommandResult<Workspace> {
    Ok(Workspace::open(root)?)
}

/// Resolve a workspace-relative script path (traversal is rejected by core).
fn script_path(ws: &Workspace, rel: &str) -> CommandResult<PathBuf> {
    let path = ws.resolve(rel)?;
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(CommandError::invalid_request(format!(
            "scripts must be Markdown files, got `{rel}`"
        )));
    }
    Ok(path)
}

/// A path that must already exist under `<workspace>/generated/`.
fn generated_path(ws: &Workspace, path: &str) -> CommandResult<PathBuf> {
    let generated = ws
        .generated_dir()
        .canonicalize()
        .map_err(|e| AppError::read(ws.generated_dir(), e))?;
    let candidate = Path::new(path);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        ws.root().join(candidate)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|e| AppError::read(&candidate, e))?;
    if !canonical.starts_with(&generated) {
        return Err(CommandError::invalid_request(format!(
            "`{path}` is outside {}",
            generated.display()
        )));
    }
    Ok(canonical)
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- platform

#[derive(Debug, Clone, Serialize)]
pub struct PlatformInfo {
    pub platform: PlatformKind,
    pub platform_label: String,
    pub can_export_ymm4: bool,
    pub can_open_ymm4: bool,
    pub ymm4_path: Option<String>,
    /// Shown instead of the export buttons when `can_export_ymm4` is false.
    pub ymm4_unavailable_reason: Option<String>,
}

/// Cheap, offline capability check for the initial render (§25). `doctor`
/// is the full report and may take a moment (it talks to VOICEVOX).
#[tauri::command(rename_all = "snake_case")]
pub fn platform_info() -> PlatformInfo {
    let platform = current_platform();
    let caps = Ymm4Exporter::new().capabilities();
    let ymm4_path = platform.find_ymm4();
    PlatformInfo {
        platform: platform.kind(),
        platform_label: platform_label(),
        can_export_ymm4: caps.available,
        can_open_ymm4: ymm4_path.is_some(),
        ymm4_path: ymm4_path.as_deref().map(path_string),
        ymm4_unavailable_reason: caps.reason,
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn doctor(
    workspace: Option<String>,
    fake_tts: bool,
    endpoint: Option<String>,
) -> CommandResult<DoctorReport> {
    let ws = match workspace.as_deref() {
        Some(root) => Workspace::open(root),
        None => Workspace::discover_cwd(),
    };
    let (endpoint, timeout, allow_remote) =
        match ws.as_ref().ok().and_then(|ws| ws.load_config().ok()) {
            Some(cfg) => (
                endpoint.unwrap_or(cfg.tts.endpoint),
                cfg.tts.timeout_secs,
                cfg.tts.allow_remote_endpoint,
            ),
            None => (
                endpoint.unwrap_or_else(|| videoforge_voicevox::DEFAULT_ENDPOINT.to_string()),
                30,
                false,
            ),
        };
    let tts = make_tts(fake_tts, &endpoint, timeout, allow_remote)?;
    let preview = FfmpegPreviewRenderer::detect();
    let exporter = Ymm4Exporter::new();
    let platform = current_platform();
    Ok(doctor::run(DoctorInput {
        workspace: ws,
        tts: tts.as_ref(),
        tts_endpoint: endpoint,
        preview: Some(&preview),
        exporter: Some(&exporter),
        platform: platform.as_ref(),
    })
    .await)
}

// ---------------------------------------------------------------- workspace

#[derive(Debug, Clone, Serialize)]
pub struct ScriptEntry {
    /// Workspace-relative, forward slashes (`scripts/sample.md`).
    pub rel: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedEntry {
    pub slug: String,
    pub output_dir: String,
    pub has_preview: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceInfo {
    pub root: String,
    pub name: String,
    pub speakers: Vec<String>,
    pub tts_endpoint: String,
    pub scripts: Vec<ScriptEntry>,
    pub generated: Vec<GeneratedEntry>,
}

fn workspace_info(ws: &Workspace) -> CommandResult<WorkspaceInfo> {
    let cfg = ws.load_config()?;
    let mut scripts: Vec<ScriptEntry> = std::fs::read_dir(ws.scripts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .map(|e| ScriptEntry {
            rel: ws.relative(&e.path()),
            name: e.file_name().to_string_lossy().into_owned(),
        })
        .collect();
    scripts.sort_by(|a, b| a.rel.cmp(&b.rel));

    let mut generated: Vec<GeneratedEntry> = std::fs::read_dir(ws.generated_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(PROJECT_FILE).is_file())
        .map(|e| GeneratedEntry {
            slug: e.file_name().to_string_lossy().into_owned(),
            has_preview: e.path().join(PREVIEW_FILE).is_file(),
            output_dir: path_string(&e.path()),
        })
        .collect();
    generated.sort_by(|a, b| a.slug.cmp(&b.slug));

    Ok(WorkspaceInfo {
        root: path_string(ws.root()),
        name: cfg.project.name.clone(),
        speakers: cfg.speakers.keys().cloned().collect(),
        tts_endpoint: cfg.tts.endpoint.clone(),
        scripts,
        generated,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn create_workspace(dir: String, name: Option<String>) -> CommandResult<WorkspaceInfo> {
    let report = core_init::init(Path::new(&dir), name.as_deref())?;
    workspace_info(&Workspace::open(&report.root)?)
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_workspace(root: String) -> CommandResult<WorkspaceInfo> {
    workspace_info(&open(&root)?)
}

#[tauri::command(rename_all = "snake_case")]
pub fn read_script(root: String, script: String) -> CommandResult<String> {
    let ws = open(&root)?;
    let path = script_path(&ws, &script)?;
    Ok(std::fs::read_to_string(&path).map_err(|e| AppError::read(&path, e))?)
}

#[tauri::command(rename_all = "snake_case")]
pub fn write_script(root: String, script: String, content: String) -> CommandResult<ScriptEntry> {
    let ws = open(&root)?;
    let path = script_path(&ws, &script)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
    }
    std::fs::write(&path, content).map_err(|e| AppError::write(&path, e))?;
    Ok(ScriptEntry {
        rel: ws.relative(&path),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn validate_script(root: String, script: String) -> CommandResult<ValidationReport> {
    let ws = open(&root)?;
    let cfg = ws.load_config()?;
    let path = script_path(&ws, &script)?;
    Ok(core_validate::validate_file(&ws, &cfg, &path))
}

// ---------------------------------------------------------------- generate

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GenerateRequest {
    #[serde(default)]
    pub no_preview: bool,
    #[serde(default)]
    pub srt_speaker: bool,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub fake_tts: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedInfo {
    pub slug: String,
    pub output_dir: String,
    pub project_path: String,
    pub preview_path: Option<String>,
    pub manifest: Manifest,
    pub project: VideoProject,
    pub warnings: Vec<String>,
}

struct EventProgress(AppHandle);

impl ProgressSink for EventProgress {
    fn on_stage(&self, stage: &GenerationStage) {
        // A closed window is the only way this fails; nothing to do about it.
        let _ = self.0.emit(PROGRESS_EVENT, stage);
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn generate(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    script: String,
    request: GenerateRequest,
) -> CommandResult<GeneratedInfo> {
    let ws = open(&root)?;
    let cfg = ws.load_config()?;
    let path = script_path(&ws, &script)?;

    let cancel = state.begin_generate().ok_or_else(|| {
        CommandError::from(AppError::Busy {
            slug: "(this window)".into(),
            lock_path: PathBuf::new(),
        })
    })?;

    let run = async {
        let endpoint = request.endpoint.clone().unwrap_or(cfg.tts.endpoint.clone());
        let tts = make_tts(
            request.fake_tts,
            &endpoint,
            cfg.tts.timeout_secs,
            cfg.tts.allow_remote_endpoint,
        )?;
        let cache = if request.no_cache { None } else { make_cache() };
        let preview: Option<Arc<dyn PreviewRenderer>> =
            Some(Arc::new(FfmpegPreviewRenderer::detect()));
        let deps = GenerateDeps {
            tts,
            cache,
            preview,
            progress: Arc::new(EventProgress(app.clone())),
        };
        let options = GenerateOptions {
            preview: if request.no_preview {
                Some(false)
            } else {
                None
            },
            srt_include_speaker: request.srt_speaker,
            cancel: cancel.clone(),
            keep_tmp_on_failure: false,
        };
        let out = core_generate::generate(&ws, &path, options, deps).await?;
        Ok::<_, CommandError>(GeneratedInfo {
            slug: out.slug,
            output_dir: path_string(&out.output_dir),
            project_path: path_string(&out.project_path),
            preview_path: out.preview_path.as_deref().map(path_string),
            manifest: out.manifest,
            project: out.project,
            warnings: out.warnings,
        })
    };
    let result = run.await;
    state.end_generate();
    result
}

#[tauri::command(rename_all = "snake_case")]
pub fn cancel_generate(state: State<'_, AppState>) -> bool {
    state.cancel_generate()
}

/// Reload a previously generated slug (after restart, or to inspect an older
/// output) without regenerating.
#[tauri::command(rename_all = "snake_case")]
pub fn load_generated(root: String, slug: String) -> CommandResult<GeneratedInfo> {
    let ws = open(&root)?;
    let output_dir = generated_path(&ws, &format!("generated/{slug}"))?;
    let project_path = output_dir.join(PROJECT_FILE);
    let project = VideoProject::load(&project_path)?;
    let manifest = Manifest::load(&output_dir.join(MANIFEST_FILE))?;
    let preview = output_dir.join(PREVIEW_FILE);
    Ok(GeneratedInfo {
        slug,
        output_dir: path_string(&output_dir),
        project_path: path_string(&project_path),
        preview_path: preview.is_file().then(|| path_string(&preview)),
        warnings: manifest.warnings.clone(),
        manifest,
        project,
    })
}

/// Raw bytes of a file under `generated/` (the preview video). Returned as a
/// binary IPC response so the frontend can build a `blob:` URL without an
/// asset-protocol scope covering the whole disk.
#[tauri::command(rename_all = "snake_case")]
pub fn read_generated_file(root: String, path: String) -> CommandResult<tauri::ipc::Response> {
    let ws = open(&root)?;
    let file = generated_path(&ws, &path)?;
    let bytes = std::fs::read(&file).map_err(|e| AppError::read(&file, e))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command(rename_all = "snake_case")]
pub fn reveal_path(path: String) -> CommandResult<()> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(CommandError::from(AppError::read(
            p,
            std::io::Error::from(std::io::ErrorKind::NotFound),
        )));
    }
    Ok(current_platform().reveal_in_file_manager(p)?)
}

// ---------------------------------------------------------------- YMM4

#[derive(Debug, Clone, Serialize)]
pub struct ExportResponse {
    pub output: String,
    pub warnings: Vec<String>,
    pub opened: bool,
}

#[tauri::command(rename_all = "snake_case")]
pub async fn export_ymm4(
    root: String,
    project_path: String,
    open_after: bool,
) -> CommandResult<ExportResponse> {
    let ws = open(&root)?;
    let project_path = generated_path(&ws, &project_path)?;
    let project_dir = project_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| ws.root().to_path_buf());
    let project = VideoProject::load(&project_path)?;
    let exporter = Ymm4Exporter::new();
    // Capability first, so macOS gets `ymm4_unavailable` before any file is
    // touched — the UI shows this as information, not as a failure (§30).
    let caps = exporter.capabilities();
    if !caps.available {
        return Err(AppError::Ymm4Unavailable(
            caps.reason
                .unwrap_or_else(|| videoforge_export_ymm4::NON_WINDOWS_MESSAGE.into()),
        )
        .into());
    }
    let result = exporter
        .export(ExportRequest {
            project: &project,
            project_path: &project_path,
            project_dir: &project_dir,
            workspace: Some(&ws),
            template: None,
            destination: None,
        })
        .await?;
    let mut warnings = result.warnings;
    let mut opened = false;
    if open_after {
        match current_platform().open_in_ymm4(&result.output) {
            Ok(()) => opened = true,
            Err(e) => warnings.push(format!("could not open YMM4: {e}")),
        }
    }
    Ok(ExportResponse {
        output: path_string(&result.output),
        warnings,
        opened,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_in_ymm4(root: String, ymmp_path: String) -> CommandResult<()> {
    let ws = open(&root)?;
    let path = generated_path(&ws, &ymmp_path)?;
    Ok(current_platform().open_in_ymm4(&path)?)
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleResponse {
    pub dir: String,
    pub zip: Option<String>,
    pub warnings: Vec<String>,
}

#[tauri::command(rename_all = "snake_case")]
pub fn bundle_ymm4(root: String, project_path: String) -> CommandResult<BundleResponse> {
    let ws = open(&root)?;
    let project_path = generated_path(&ws, &project_path)?;
    let result = create_bundle(
        &project_path,
        Some(&ws),
        BundleOptions {
            out_dir: None,
            template: None,
            zip: true,
            // Re-bundling the same slug is the common GUI flow; `force` only
            // ever replaces a directory that already is a VideoForge bundle.
            force: true,
        },
    )?;
    Ok(BundleResponse {
        dir: path_string(&result.dir),
        zip: result.zip.as_deref().map(path_string),
        warnings: result.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        core_init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn script_paths_are_confined_to_the_workspace_and_markdown() {
        let (_dir, ws) = workspace();
        assert!(script_path(&ws, "scripts/sample.md").is_ok());
        let err = script_path(&ws, "../outside.md").unwrap_err();
        assert_eq!(err.code, "invalid_workspace_path");
        let err = script_path(&ws, "videoforge.yaml").unwrap_err();
        assert_eq!(err.code, "invalid_request");
    }

    #[test]
    fn generated_paths_must_live_under_generated() {
        let (_dir, ws) = workspace();
        let out = ws.generated_dir().join("sample");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(PREVIEW_FILE), b"x").unwrap();

        let ok = generated_path(&ws, "generated/sample/preview.mp4").unwrap();
        assert!(ok.ends_with("preview.mp4"));
        let abs = out.join(PREVIEW_FILE);
        assert!(generated_path(&ws, &path_string(&abs)).is_ok());

        let err = generated_path(&ws, "videoforge.yaml").unwrap_err();
        assert_eq!(err.code, "invalid_request");
        let err = generated_path(&ws, "generated/../videoforge.yaml").unwrap_err();
        assert_eq!(err.code, "invalid_request");
        let err = generated_path(&ws, "generated/missing.mp4").unwrap_err();
        assert_eq!(err.code, "file_read_failed");
    }

    #[test]
    fn workspace_info_lists_scripts_and_generated_outputs() {
        let (_dir, ws) = workspace();
        let info = workspace_info(&ws).unwrap();
        assert_eq!(info.name, "t");
        assert_eq!(info.scripts.len(), 1);
        assert_eq!(info.scripts[0].rel, "scripts/sample.md");
        assert!(info.generated.is_empty());
        assert!(info.speakers.contains(&"reimu".to_string()));
    }
}
