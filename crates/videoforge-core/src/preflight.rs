//! Target-specific checks performed before an expensive generation or render.
//! Environment-wide capability checks remain in `doctor`; this module answers
//! whether one script or canonical VideoProject is ready to process.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use videoforge_project::validation::validate_project;
use videoforge_project::VideoProject;

use crate::assets::build_registry;
use crate::tts::TtsEngine;
use crate::presentation::{self, DiagnosticLevel, MarpCli, PresentationRenderer};
use crate::{character, validate, AppError, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    Pass,
    Warning,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightCheck {
    pub code: String,
    pub status: PreflightStatus,
    pub path: Option<String>,
    pub detail: String,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    pub target: String,
    pub target_kind: String,
    pub status: PreflightStatus,
    pub checks: Vec<PreflightCheck>,
    pub duration_ms: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}

impl PreflightReport {
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == PreflightStatus::Failure)
    }

    pub fn has_warnings(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == PreflightStatus::Warning)
    }

    pub fn status(&self) -> PreflightStatus {
        self.status
    }
}

fn overall_status(checks: &[PreflightCheck]) -> PreflightStatus {
    if checks
        .iter()
        .any(|check| check.status == PreflightStatus::Failure)
    {
        PreflightStatus::Failure
    } else if checks
        .iter()
        .any(|check| check.status == PreflightStatus::Warning)
    {
        PreflightStatus::Warning
    } else {
        PreflightStatus::Pass
    }
}

fn check(
    code: &str,
    status: PreflightStatus,
    path: Option<String>,
    detail: impl Into<String>,
    remediation: Option<&str>,
) -> PreflightCheck {
    PreflightCheck {
        code: code.into(),
        status,
        path,
        detail: detail.into(),
        remediation: remediation.map(str::to_string),
    }
}

/// Inspect one script or `project.vfp.json`. This performs no synthesis and
/// writes no project files. A real TTS engine is queried only for health and
/// speaker identity; the fake engine explicitly reports that as unverified.
pub async fn run(
    workspace: &Workspace,
    target: &Path,
    tts: &dyn TtsEngine,
) -> Result<PreflightReport, AppError> {
    run_with_presentation(workspace, target, tts, None).await
}

/// Run the existing target checks and, only when explicitly requested,
/// validate the Marp source/tool needed by `generate --presentation`.
pub async fn run_with_presentation(
    workspace: &Workspace,
    target: &Path,
    tts: &dyn TtsEngine,
    presentation_source: Option<&Path>,
) -> Result<PreflightReport, AppError> {
    let is_project = target
        .file_name()
        .is_some_and(|name| name == "project.vfp.json")
        || target.extension().is_some_and(|extension| extension == "json");
    let mut report = if is_project {
        project_preflight(workspace, target, tts).await
    } else {
        script_preflight(workspace, target, tts).await
    }?;
    if let Some(source) = presentation_source {
        append_presentation_checks(workspace, target, source, is_project, &mut report.checks);
        report.status = overall_status(&report.checks);
    }
    Ok(report)
}

fn append_presentation_checks(
    workspace: &Workspace,
    target: &Path,
    source: &Path,
    is_project: bool,
    checks: &mut Vec<PreflightCheck>,
) {
    let mut slide_count = None;
    match presentation::validate(workspace, source) {
        Ok(report) => {
            slide_count = Some(report.slide_count);
            let is_valid = report.is_valid();
            for diagnostic in report.diagnostics {
                checks.push(check(
                    &diagnostic.code,
                    match diagnostic.level {
                        DiagnosticLevel::Warning => PreflightStatus::Warning,
                        DiagnosticLevel::Failure => PreflightStatus::Failure,
                    },
                    diagnostic.path,
                    diagnostic.detail,
                    Some(&diagnostic.remediation),
                ));
            }
            if is_valid {
                checks.push(check(
                    "presentation_source",
                    PreflightStatus::Pass,
                    Some(workspace.relative(source)),
                    format!("valid Marp Markdown with {} slide(s)", report.slide_count),
                    None,
                ));
            }
        }
        Err(error) => checks.push(check(
            "presentation_source",
            PreflightStatus::Failure,
            Some(workspace.relative(source)),
            error.to_string(),
            Some("restore the source and fix its Marp Markdown before generate"),
        )),
    }

    match MarpCli::detect().availability() {
        Ok(info) => checks.push(check(
            "marp",
            PreflightStatus::Pass,
            Some(info.executable),
            info.version,
            None,
        )),
        Err(error) => checks.push(check(
            "marp_unavailable",
            PreflightStatus::Failure,
            Some("VIDEOFORGE_MARP".into()),
            error.to_string(),
            Some("install the Marp CLI executable and a supported browser, or set VIDEOFORGE_MARP"),
        )),
    }

    if !is_project {
        if let Some(slides) = slide_count {
            if let Ok(config) = workspace.load_config() {
                let script = validate::validate_file(workspace, &config, target);
                let narration_segments = script.dialogues.len();
                checks.push(check(
                    "presentation_mapping",
                    if slides == narration_segments {
                        PreflightStatus::Pass
                    } else {
                        PreflightStatus::Failure
                    },
                    Some(workspace.relative(source)),
                    format!(
                        "{slides} slide(s), {narration_segments} narration segment(s)"
                    ),
                    if slides == narration_segments {
                        None
                    } else {
                        Some("make slide count exactly match script dialogue count; P0 never silently adjusts")
                    },
                ));
            }
        }
    }
}

async fn script_preflight(
    workspace: &Workspace,
    target: &Path,
    tts: &dyn TtsEngine,
) -> Result<PreflightReport, AppError> {
    let mut config = workspace.load_config()?;
    let mut checks = Vec::new();
    check_tts(tts, &mut checks).await;

    if tts.id() != "fake" {
        match character::resolve_character_voices(&mut config, workspace, tts).await {
            Ok(()) => checks.push(check(
                "speaker_identity",
                PreflightStatus::Pass,
                Some("speakers".into()),
                "all configured speaker identities resolve in the connected engine",
                None,
            )),
            Err(error) => checks.push(check(
                "speaker_identity",
                PreflightStatus::Failure,
                Some("speakers".into()),
                error.to_string(),
                Some("correct the speaker/style mapping or character profile before generate"),
            )),
        }
    }

    let validation = validate::validate_file(workspace, &config, target);
    checks.extend(validation.errors.iter().map(|issue| {
        check(
            "script_validation",
            PreflightStatus::Failure,
            issue.line.map(|line| format!("line:{line}")),
            issue.message.clone(),
            Some("fix the reported script or asset reference"),
        )
    }));
    checks.extend(validation.warnings.iter().map(|issue| {
        check(
            "script_warning",
            PreflightStatus::Warning,
            issue.line.map(|line| format!("line:{line}")),
            issue.message.clone(),
            Some("review before publishing"),
        )
    }));
    if validation.errors.is_empty() {
        checks.push(check(
            "script_validation",
            PreflightStatus::Pass,
            Some(workspace.relative(target)),
            format!(
                "{} dialogue(s), {} character(s)",
                validation.dialogues.len(),
                validation.total_chars
            ),
            None,
        ));
    }

    check_preview_files(workspace, &config, &mut checks);
    check_character_assets(workspace, &config, &mut checks);
    let duration_ms = if validation.total_chars == 0 {
        None
    } else {
        Some((validation.total_chars as u64).saturating_mul(200))
    };
    let estimate = duration_ms.map(estimate_output_bytes);
    check_disk_space(workspace, estimate, &mut checks);

    let status = overall_status(&checks);
    Ok(PreflightReport {
        target: workspace.relative(target),
        target_kind: "script".into(),
        status,
        checks,
        duration_ms,
        estimated_output_bytes: estimate,
    })
}

async fn project_preflight(
    workspace: &Workspace,
    target: &Path,
    tts: &dyn TtsEngine,
) -> Result<PreflightReport, AppError> {
    let config = workspace.load_config()?;
    let mut checks = Vec::new();
    check_tts(tts, &mut checks).await;
    let project = match VideoProject::load(target) {
        Ok(project) => project,
        Err(error) => {
            checks.push(check(
                "project_parse",
                PreflightStatus::Failure,
                Some(target.display().to_string()),
                error.to_string(),
                Some("repair or regenerate project.vfp.json"),
            ));
            let status = overall_status(&checks);
            return Ok(PreflightReport {
                target: workspace.relative(target),
                target_kind: "project".into(),
                status,
                checks,
                duration_ms: None,
                estimated_output_bytes: None,
            });
        }
    };

    let validation = validate_project(&project);
    checks.extend(validation.errors.into_iter().map(|issue| {
        check(
            &issue.code,
            PreflightStatus::Failure,
            Some(issue.path),
            issue.message,
            Some("repair or regenerate project.vfp.json"),
        )
    }));
    checks.extend(validation.warnings.into_iter().map(|issue| {
        check(
            &issue.code,
            PreflightStatus::Warning,
            Some(issue.path),
            issue.message,
            Some("review the canonical project before rendering"),
        )
    }));

    let loaded_manifest = match character::load_manifest(&config, workspace) {
        Ok(loaded) => loaded,
        Err(error) => {
            checks.push(check(
                "character_manifest",
                PreflightStatus::Failure,
                config.character_manifest.clone(),
                error.to_string(),
                Some("correct the character manifest path or contents"),
            ));
            None
        }
    };
    let referenced_characters: std::collections::BTreeSet<&str> = project
        .character_performance_clips()
        .iter()
        .map(|clip| clip.character.as_str())
        .collect();
    if !referenced_characters.is_empty() && loaded_manifest.is_none() {
        checks.push(check(
            "character_manifest_required",
            PreflightStatus::Failure,
            Some("character_manifest".into()),
            format!(
                "project references character(s) {} but no resolvable character manifest is \
                 configured",
                referenced_characters.iter().copied().collect::<Vec<_>>().join(", ")
            ),
            Some("configure character_manifest and matching speaker character_id entries"),
        ));
    }
    if let Some(loaded) = &loaded_manifest {
        for character_id in &referenced_characters {
            if loaded.manifest.find(character_id).is_none() {
                checks.push(check(
                    "character_not_found",
                    PreflightStatus::Failure,
                    Some((*character_id).into()),
                    "project character is absent from the configured manifest",
                    Some("restore the manifest entry used when this project was generated"),
                ));
            }
        }
    }
    let manifest_ref = loaded_manifest.as_ref().map(|loaded| {
        (
            &loaded.manifest,
            loaded.path.parent().unwrap_or_else(|| Path::new(".")),
        )
    });
    let project_dir = target.parent().unwrap_or_else(|| Path::new("."));
    let registry = build_registry(&project, project_dir, manifest_ref);
    for asset in registry.missing() {
        for file in asset.files.iter().filter(|file| !file.exists) {
            checks.push(check(
                "missing_asset",
                PreflightStatus::Failure,
                Some(file.path.clone()),
                format!("{} is missing {}", asset.id, file.role),
                Some("restore the file or update the asset reference"),
            ));
        }
    }
    if registry.missing().is_empty() {
        checks.push(check(
            "project_assets",
            PreflightStatus::Pass,
            Some(project_dir.display().to_string()),
            format!("all {} registered asset(s) are present", registry.assets.len()),
            None,
        ));
    }

    check_preview_files(workspace, &config, &mut checks);
    check_character_assets(workspace, &config, &mut checks);
    let duration_ms = Some(project.total_duration_ms());
    let estimate = duration_ms.map(estimate_output_bytes);
    check_disk_space(workspace, estimate, &mut checks);

    let status = overall_status(&checks);
    Ok(PreflightReport {
        target: workspace.relative(target),
        target_kind: "project".into(),
        status,
        checks,
        duration_ms,
        estimated_output_bytes: estimate,
    })
}

async fn check_tts(tts: &dyn TtsEngine, checks: &mut Vec<PreflightCheck>) {
    if tts.id() == "fake" {
        checks.push(check(
            "tts_unverified",
            PreflightStatus::Warning,
            None,
            "fake TTS selected; VOICEVOX reachability and installed speakers were not verified",
            Some("run preflight without --fake-tts before a production generate"),
        ));
        return;
    }
    match tts.health().await {
        Ok(info) => checks.push(check(
            "tts_connection",
            PreflightStatus::Pass,
            None,
            format!("{} {} at {}", info.engine, info.version, info.endpoint),
            None,
        )),
        Err(error) => checks.push(check(
            "tts_connection",
            PreflightStatus::Failure,
            None,
            error.to_string(),
            Some("start VOICEVOX or correct the configured endpoint"),
        )),
    }
}

fn check_preview_files(
    workspace: &Workspace,
    config: &crate::Config,
    checks: &mut Vec<PreflightCheck>,
) {
    for (code, configured, allow_absolute) in [
        ("preview_background", config.preview.background.as_deref(), false),
        ("preview_font", config.preview.font.as_deref(), true),
    ] {
        let Some(value) = configured else { continue };
        let resolved: Result<PathBuf, AppError> = if allow_absolute {
            workspace.resolve_allow_absolute(value)
        } else {
            workspace.resolve(value)
        };
        match resolved {
            Ok(path) if path.is_file() => checks.push(check(
                code,
                PreflightStatus::Pass,
                Some(value.into()),
                "file is readable",
                None,
            )),
            Ok(_) => checks.push(check(
                code,
                PreflightStatus::Failure,
                Some(value.into()),
                "configured file does not exist",
                Some("restore the file or correct videoforge.yaml"),
            )),
            Err(error) => checks.push(check(
                code,
                PreflightStatus::Failure,
                Some(value.into()),
                error.to_string(),
                Some("use an allowed path in videoforge.yaml"),
            )),
        }
    }
}

fn check_character_assets(
    workspace: &Workspace,
    config: &crate::Config,
    checks: &mut Vec<PreflightCheck>,
) {
    let loaded = match character::load_manifest(config, workspace) {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return,
        Err(error) => {
            checks.push(check(
                "character_manifest",
                PreflightStatus::Failure,
                config.character_manifest.clone(),
                error.to_string(),
                Some("correct the character manifest path or contents"),
            ));
            return;
        }
    };
    let manifest_dir = loaded.path.parent().unwrap_or_else(|| Path::new("."));
    let mut character_ids: Vec<&str> = config
        .speakers
        .values()
        .filter_map(|speaker| speaker.character_id.as_deref())
        .collect();
    character_ids.sort_unstable();
    character_ids.dedup();
    for character_id in character_ids {
        let Some(character) = loaded.manifest.find(character_id) else {
            checks.push(check(
                "character_not_found",
                PreflightStatus::Failure,
                Some(character_id.into()),
                "configured character is absent from the manifest",
                Some("correct speakers.*.character_id or add the manifest entry"),
            ));
            continue;
        };
        let Some(model) = &character.model else { continue };
        if !model.is_png_lipsync() {
            continue;
        }
        match videoforge_character::png_lipsync::load_png_lipsync_assets(model, manifest_dir) {
            Ok(assets) if assets.dimensions_mismatched() => checks.push(check(
                "png_lipsync_dimensions",
                PreflightStatus::Warning,
                Some(character_id.into()),
                format!(
                    "closed={}x{}, half={}x{}, open={}x{}; mouth frames may jump",
                    assets.closed.width,
                    assets.closed.height,
                    assets.half.width,
                    assets.half.height,
                    assets.open.width,
                    assets.open.height
                ),
                Some("export all three PNGs from the same canvas size"),
            )),
            Ok(_) => checks.push(check(
                "png_lipsync_assets",
                PreflightStatus::Pass,
                Some(character_id.into()),
                "three transparent PNG mouth frames have matching dimensions",
                None,
            )),
            Err(error) => checks.push(check(
                "png_lipsync_assets",
                PreflightStatus::Failure,
                Some(character_id.into()),
                error.to_string(),
                Some("provide readable transparent closed/half/open PNGs"),
            )),
        }
    }
}

fn estimate_output_bytes(duration_ms: u64) -> u64 {
    // Conservative planning estimate for the default 1080p preview: 2 Mbps
    // video+audio plus 25% mux/container headroom. It is explicitly an
    // estimate, not an encoder promise.
    duration_ms.saturating_mul(2_000_000).saturating_mul(5) / 4 / 8 / 1000
}

fn check_disk_space(
    workspace: &Workspace,
    estimated: Option<u64>,
    checks: &mut Vec<PreflightCheck>,
) {
    let Some(required) = estimated else { return };
    let output = workspace.generated_dir();
    match fs4::available_space(&output) {
        Ok(available) if available >= required => checks.push(check(
            "output_capacity",
            PreflightStatus::Pass,
            Some(output.display().to_string()),
            format!(
                "estimated {} MiB; {} MiB available",
                required / 1_048_576,
                available / 1_048_576
            ),
            None,
        )),
        Ok(available) => checks.push(check(
            "output_capacity",
            PreflightStatus::Failure,
            Some(output.display().to_string()),
            format!(
                "estimated {} MiB; only {} MiB available",
                required / 1_048_576,
                available / 1_048_576
            ),
            Some("free disk space or choose a shorter/lower-resolution render"),
        )),
        Err(error) => checks.push(check(
            "output_capacity_unverified",
            PreflightStatus::Warning,
            Some(output.display().to_string()),
            error.to_string(),
            Some("verify free disk space manually"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{init, tts::FakeTtsEngine};

    #[tokio::test]
    async fn valid_script_with_fake_tts_is_warning_not_failure() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("preflight")).unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        let report = run(
            &workspace,
            &dir.path().join("scripts/sample.md"),
            &FakeTtsEngine::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.target_kind, "script");
        assert!(!report.has_failures());
        assert_eq!(report.status(), PreflightStatus::Warning);
    }

    #[tokio::test]
    async fn project_reports_all_missing_assets() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("preflight")).unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        let project_dir = dir.path().join("generated/test");
        std::fs::create_dir_all(&project_dir).unwrap();
        let mut project = VideoProject::new("test", "Test", Default::default());
        project.tracks.push(videoforge_project::Track {
            id: "background".into(),
            kind: videoforge_project::TrackKind::Background,
            clips: vec![videoforge_project::Clip::Background(
                videoforge_project::BackgroundClip {
                    id: "missing-background".into(),
                    source: videoforge_project::RelativeAssetPath::new("assets/missing.png")
                        .unwrap(),
                    start_ms: 0,
                    duration_ms: 1_000,
                    extra: Default::default(),
                },
            )],
        });
        let path = project_dir.join("project.vfp.json");
        project.save(&path).unwrap();
        let report = run(&workspace, &path, &FakeTtsEngine::default())
            .await
            .unwrap();
        assert!(report.has_failures());
        assert!(report
            .checks
            .iter()
            .any(|item| item.code == "missing_asset"));
    }
}
