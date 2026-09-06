//! YMM4 exporter (design §20–§23).
//!
//! **Template patch** strategy: VideoForge never re-implements the YMM4
//! schema. It loads a template `.ymmp` authored in YMM4, finds prototype
//! items by their `Remark`, clones them per clip, patches only the fields it
//! understands (`Text`, `Frame`, `Length`, `FilePath`, `Remark`) and writes
//! the result back with every other property preserved.
//!
//! Materializing `.ymmp` is Windows-only because asset paths must be Windows
//! absolute paths that exist on the machine running YMM4. On other platforms
//! use [`bundle`] to create a portable handoff.

pub mod bundle;
pub mod patch;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use videoforge_core::export::{ExportCapabilities, ExportRequest, ExportResult, ProjectExporter};
use videoforge_core::AppError;

pub use patch::{
    patch_template, PatchOutput, PROTO_AUDIO, PROTO_CAPTION, PROTO_CHARACTER, PROTO_PREFIX,
};

pub const EXPORT_DIR: &str = "ymm4";
pub const EXPORT_FILE: &str = "project.ymmp";
/// Template copied into a handoff bundle.
pub const BUNDLE_TEMPLATE_FILE: &str = "template.ymmp";

pub const NON_WINDOWS_MESSAGE: &str =
    "YMM4 materialization requires Windows.\n\nUse:\nvideoforge bundle ymm4 <project.vfp.json>";
/// One-line variant for capability listings.
pub const NON_WINDOWS_REASON: &str =
    "YMM4 materialization requires Windows (use `videoforge bundle ymm4 <project.vfp.json>` to hand off)";

#[derive(Debug, Clone, Default)]
pub struct Ymm4Exporter {
    /// Allow materialization on non-Windows hosts. For CI/test coverage of
    /// the template-patch logic on Linux/macOS runners only (see
    /// `videoforge-cli/tests/cli.rs` and `docs/testing/ymm4-manual-e2e.md`):
    /// the resulting paths are Windows paths that do not exist on the host
    /// running this, so they are never valid for a real YMM4 install. Not a
    /// supported production workflow; the CLI hides the corresponding
    /// `--force` flag from `--help`.
    pub force_non_windows: bool,
}

impl Ymm4Exporter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_platform_supported(&self) -> bool {
        cfg!(target_os = "windows") || self.force_non_windows
    }

    /// Template lookup order: explicit → `template.ymmp` next to the project
    /// (handoff bundle) → workspace config.
    pub fn resolve_template(request: &ExportRequest<'_>) -> Result<PathBuf, AppError> {
        if let Some(t) = request.template {
            return Ok(t.to_path_buf());
        }
        let sibling = request.project_dir.join(BUNDLE_TEMPLATE_FILE);
        if sibling.is_file() {
            return Ok(sibling);
        }
        if let Some(ws) = request.workspace {
            let cfg = ws.load_config()?;
            let path = ws.resolve(&cfg.export.ymm4.template)?;
            if path.is_file() {
                return Ok(path);
            }
            return Err(AppError::InvalidTemplate {
                path,
                reason: "template not found; create it in YMM4 (see templates/ymm4/README.md) or pass --template".into(),
            });
        }
        Err(AppError::InvalidTemplate {
            path: sibling,
            reason: "no template found; pass --template or run inside a workspace".into(),
        })
    }
}

/// Read a `.ymmp` file, tolerating a UTF-8 BOM. Returns `(json, had_bom)`.
pub fn read_template(path: &Path) -> Result<(serde_json::Value, bool), AppError> {
    let bytes = std::fs::read(path).map_err(|e| AppError::read(path, e))?;
    let had_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let text = if had_bom { &bytes[3..] } else { &bytes[..] };
    let value: serde_json::Value =
        serde_json::from_slice(text).map_err(|e| AppError::InvalidTemplate {
            path: path.to_path_buf(),
            reason: format!("not valid JSON: {e}"),
        })?;
    if !value.is_object() {
        return Err(AppError::InvalidTemplate {
            path: path.to_path_buf(),
            reason: "top-level JSON value must be an object".into(),
        });
    }
    Ok((value, had_bom))
}

pub fn write_ymmp(path: &Path, value: &serde_json::Value, with_bom: bool) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::serialization("ymm4 project", e))?;
    let mut bytes = Vec::with_capacity(json.len() + 3);
    if with_bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(json.as_bytes());
    std::fs::write(path, bytes).map_err(|e| AppError::write(path, e))
}

/// Convert a project-relative asset path into a Windows absolute path rooted
/// at `project_dir`.
pub fn materialize_windows_path(project_dir: &Path, relative: &str) -> String {
    let root = absolute_root(project_dir);
    let root = root.to_string_lossy().replace('/', "\\");
    let root = root.trim_end_matches('\\');
    let rel = relative.replace('/', "\\");
    format!("{root}\\{rel}")
}

fn absolute_root(dir: &Path) -> PathBuf {
    let abs = dir
        .canonicalize()
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|c| c.join(dir)))
        .unwrap_or_else(|| dir.to_path_buf());
    // std::fs::canonicalize on Windows yields `\\?\C:\...`; YMM4 wants `C:\...`.
    let s = abs.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => abs,
    }
}

#[async_trait]
impl ProjectExporter for Ymm4Exporter {
    fn id(&self) -> &'static str {
        "ymm4"
    }

    fn capabilities(&self) -> ExportCapabilities {
        let available = self.is_platform_supported();
        ExportCapabilities {
            available,
            reason: (!available).then(|| NON_WINDOWS_REASON.to_string()),
            can_open: cfg!(target_os = "windows"),
        }
    }

    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, AppError> {
        if !self.is_platform_supported() {
            return Err(AppError::Ymm4Unavailable(NON_WINDOWS_MESSAGE.into()));
        }
        let template_path = Self::resolve_template(&request)?;
        let (template, had_bom) = read_template(&template_path)?;

        let output = request
            .destination
            .map(Path::to_path_buf)
            .unwrap_or_else(|| request.project_dir.join(EXPORT_DIR).join(EXPORT_FILE));
        let output_abs = materialize_windows_path(
            output.parent().unwrap_or(request.project_dir),
            &output
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| EXPORT_FILE.into()),
        );

        // Verify referenced assets exist before writing anything.
        let mut warnings = Vec::new();
        for asset in request.project.referenced_assets() {
            let path = asset.resolve(request.project_dir);
            if !path.is_file() {
                warnings.push(format!(
                    "asset not found: {} (YMM4 will show it as missing)",
                    path.display()
                ));
            }
        }

        let patched = patch_template(
            template,
            request.project,
            &template_path,
            &|rel| materialize_windows_path(request.project_dir, rel),
            Some(&output_abs),
        )?;
        warnings.extend(patched.warnings);
        write_ymmp(&output, &patched.value, had_bom)?;
        Ok(ExportResult { output, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_materialization_shape() {
        let dir = tempfile::tempdir().unwrap();
        let p = materialize_windows_path(dir.path(), "assets/audio/001.wav");
        assert!(p.ends_with(r"\assets\audio\001.wav"), "{p}");
        assert!(!p.contains('/'));
    }

    #[test]
    fn capabilities_reflect_platform() {
        let exp = Ymm4Exporter::new();
        let caps = exp.capabilities();
        assert_eq!(caps.available, cfg!(target_os = "windows"));
        if !caps.available {
            assert!(caps.reason.unwrap().contains("bundle"));
        }
        let forced = Ymm4Exporter {
            force_non_windows: true,
        };
        assert!(forced.capabilities().available);
    }
}
