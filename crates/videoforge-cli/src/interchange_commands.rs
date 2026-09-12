use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use videoforge_core::export::{ExportRequest, ProjectExporter};
use videoforge_core::project::VideoProject;
use videoforge_export_interchange::InterchangeExporter;

pub async fn export(
    project_path: PathBuf,
    out: Option<PathBuf>,
    format: &str,
    json_output: bool,
) -> anyhow::Result<ExitCode> {
    let project_path = project_path
        .canonicalize()
        .with_context(|| format!("project not found: {}", project_path.display()))?;
    let project_dir = project_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let project = VideoProject::load(&project_path)?;
    let exporter = match format {
        "fcpxml" => InterchangeExporter::fcpxml(),
        "otio" => InterchangeExporter::otio(),
        other => anyhow::bail!("unsupported interchange format: {other}"),
    };
    let result = exporter
        .export(ExportRequest {
            project: &project,
            project_path: &project_path,
            project_dir: &project_dir,
            workspace: None,
            template: None,
            destination: out.as_deref(),
        })
        .await?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"ok":true,"format":format,"output":result.output,"warnings":result.warnings})
            )?
        );
    } else {
        println!("Exported {}", result.output.display());
        for warning in result.warnings {
            println!("warning: {warning}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
