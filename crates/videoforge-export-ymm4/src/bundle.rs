//! Portable YMM4 handoff bundle (design §7.6, §23; VF-070..072).
//!
//! Created on any platform, consumed on Windows with
//! `videoforge export ymm4 <bundle>/project.vfp.json`. Asset paths inside the
//! bundle stay relative, so the Windows exporter re-materializes them.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use videoforge_core::manifest::Manifest;
use videoforge_core::platform::platform_label;
use videoforge_core::project::VideoProject;
use videoforge_core::{AppError, Workspace, GENERATOR_VERSION};

use crate::BUNDLE_TEMPLATE_FILE;

pub const BUNDLE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const BUNDLE_KIND: &str = "videoforge-ymm4-handoff";

#[derive(Debug, Clone, Default)]
pub struct BundleOptions {
    /// Directory to create. Defaults to `<project_dir>/../<slug>-ymm4-bundle`.
    pub out_dir: Option<PathBuf>,
    /// Explicit template path.
    pub template: Option<PathBuf>,
    /// Also produce `<out_dir>.zip`.
    pub zip: bool,
    /// Overwrite an existing `out_dir`. Only ever removes a directory that
    /// both passes [`ensure_safe_to_overwrite`] (not a protected directory)
    /// and already looks like a VideoForge-generated bundle.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub kind: String,
    pub generator_version: String,
    pub created_at: String,
    pub created_on: String,
    pub project_id: String,
    pub project: String,
    pub template: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub assets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_manifest: Option<Manifest>,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleResult {
    pub dir: PathBuf,
    pub zip: Option<PathBuf>,
    pub warnings: Vec<String>,
}

pub fn create_bundle(
    project_path: &Path,
    workspace: Option<&Workspace>,
    options: BundleOptions,
) -> Result<BundleResult, AppError> {
    let project = VideoProject::load(project_path)?;
    let project_dir = project_path.parent().unwrap_or(Path::new("."));
    let slug = project_dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| project.id.clone());
    let out_dir = options.out_dir.unwrap_or_else(|| {
        project_dir
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{slug}-ymm4-bundle"))
    });
    let mut warnings = Vec::new();

    if out_dir.exists() {
        if !options.force {
            return Err(AppError::UnsafeOverwrite {
                path: out_dir.clone(),
                reason:
                    "directory already exists; pass --force to overwrite a VideoForge-generated bundle"
                        .into(),
            });
        }
        ensure_safe_to_overwrite(&out_dir, workspace)?;
        std::fs::remove_dir_all(&out_dir).map_err(|e| AppError::write(&out_dir, e))?;
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| AppError::write(&out_dir, e))?;

    // project.vfp.json
    let project_file = project_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project.vfp.json".into());
    copy(project_path, &out_dir.join(&project_file))?;

    // source.md (optional)
    let source = copy_optional(&project_dir.join("source.md"), &out_dir.join("source.md"))?;

    // template.ymmp (optional but strongly recommended)
    let template_src = options
        .template
        .clone()
        .or_else(|| {
            let sibling = project_dir.join(BUNDLE_TEMPLATE_FILE);
            sibling.is_file().then_some(sibling)
        })
        .or_else(|| {
            workspace.and_then(|ws| {
                let cfg = ws.load_config().ok()?;
                let p = ws.resolve(&cfg.export.ymm4.template).ok()?;
                p.is_file().then_some(p)
            })
        });
    let template = match template_src {
        Some(src) => {
            copy(&src, &out_dir.join(BUNDLE_TEMPLATE_FILE))?;
            Some(BUNDLE_TEMPLATE_FILE.to_string())
        }
        None => {
            warnings.push(
                "no YMM4 template found; the Windows side must pass --template when exporting"
                    .into(),
            );
            None
        }
    };

    // assets
    let mut assets = Vec::new();
    for asset in project.referenced_assets() {
        let src = asset.resolve(project_dir);
        let dst = asset.resolve(&out_dir);
        if src.is_file() {
            copy(&src, &dst)?;
            assets.push(asset.as_str().to_string());
        } else {
            warnings.push(format!("asset missing, not bundled: {}", asset.as_str()));
        }
    }

    let generated_manifest = Manifest::load(&project_dir.join("manifest.json")).ok();
    let manifest = BundleManifest {
        schema_version: BUNDLE_MANIFEST_SCHEMA_VERSION,
        kind: BUNDLE_KIND.into(),
        generator_version: GENERATOR_VERSION.into(),
        created_at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        created_on: platform_label(),
        project_id: project.id.clone(),
        project: project_file.clone(),
        template,
        source,
        assets,
        generated_manifest,
        instructions: format!(
            "On Windows run: videoforge export ymm4 <this folder>/{project_file}  (add --template <path> if template.ymmp is missing)"
        ),
    };
    let manifest_path = out_dir.join("manifest.json");
    let json =
        serde_json::to_string_pretty(&manifest).map_err(|e| AppError::Other(e.to_string()))?;
    std::fs::write(&manifest_path, json).map_err(|e| AppError::write(&manifest_path, e))?;

    let zip = if options.zip {
        let zip_path = out_dir.with_extension("zip");
        zip_dir(&out_dir, &zip_path)?;
        Some(zip_path)
    } else {
        None
    };

    Ok(BundleResult {
        dir: out_dir,
        zip,
        warnings,
    })
}

/// True when `dir` looks like a handoff bundle (used by the CLI for hints).
pub fn is_bundle_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<BundleManifest>(&t).ok())
        .is_some_and(|m| m.kind == BUNDLE_KIND)
}

/// Guards `--force` bundle overwrites (VF-070..072 hardening).
///
/// Refuses to remove: a filesystem root, the user's home directory, the
/// current workspace root or its `generated/` directory, and anything that
/// does not already look like a VideoForge-generated bundle (per
/// [`is_bundle_dir`]) — so `--force` can only ever clobber a previous bundle
/// this tool created, never an arbitrary directory a mistyped `--out` points
/// at.
fn ensure_safe_to_overwrite(out_dir: &Path, workspace: Option<&Workspace>) -> Result<(), AppError> {
    let refuse = |reason: String| {
        Err(AppError::UnsafeOverwrite {
            path: out_dir.to_path_buf(),
            reason,
        })
    };
    let canonical = match out_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => return refuse(format!("cannot inspect existing directory: {e}")),
    };

    if canonical.parent().is_none() {
        return refuse("refusing to remove a filesystem root".into());
    }

    let mut protected: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        protected.push(home);
    }
    if let Some(ws) = workspace {
        protected.push(ws.root().to_path_buf());
        protected.push(ws.generated_dir());
    }
    for p in &protected {
        let Ok(p) = p.canonicalize() else { continue };
        if canonical == p || p.starts_with(&canonical) {
            return refuse(format!(
                "refusing to remove a protected directory ({})",
                p.display()
            ));
        }
    }

    if !is_bundle_dir(out_dir) {
        return refuse(
            "does not look like a VideoForge-generated bundle (no matching manifest.json); \
             refusing to overwrite"
                .into(),
        );
    }
    Ok(())
}

fn copy(src: &Path, dst: &Path) -> Result<(), AppError> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
    }
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|e| AppError::FileReadFailed {
            path: src.to_path_buf(),
            source: e,
        })
}

fn copy_optional(src: &Path, dst: &Path) -> Result<Option<String>, AppError> {
    if src.is_file() {
        copy(src, dst)?;
        Ok(dst.file_name().map(|s| s.to_string_lossy().into_owned()))
    } else {
        Ok(None)
    }
}

fn zip_dir(dir: &Path, zip_path: &Path) -> Result<(), AppError> {
    let file = std::fs::File::create(zip_path).map_err(|e| AppError::write(zip_path, e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let root_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bundle".into());
    let io_err = |e: std::io::Error| AppError::write(zip_path, e);
    let zip_err = |e: zip::result::ZipError| AppError::Other(format!("zip: {e}"));

    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.map_err(|e| AppError::Other(format!("walk: {e}")))?;
        let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path());
        let name = std::iter::once(root_name.as_str())
            .chain(
                rel.components()
                    .map(|c| c.as_os_str().to_str().unwrap_or("")),
            )
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        if entry.file_type().is_dir() {
            if !rel.as_os_str().is_empty() {
                zip.add_directory(format!("{name}/"), options)
                    .map_err(zip_err)?;
            }
        } else {
            zip.start_file(name, options).map_err(zip_err)?;
            let bytes = std::fs::read(entry.path()).map_err(|e| AppError::read(entry.path(), e))?;
            zip.write_all(&bytes).map_err(io_err)?;
        }
    }
    zip.finish().map_err(zip_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use videoforge_core::project::{
        AudioClip, Clip, RelativeAssetPath, Track, TrackKind, VideoSettings,
    };

    #[test]
    fn creates_bundle_with_assets_and_zip() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("generated").join("sample");
        std::fs::create_dir_all(project_dir.join("assets/audio")).unwrap();
        std::fs::write(project_dir.join("assets/audio/001.wav"), b"RIFF").unwrap();
        std::fs::write(project_dir.join("source.md"), "a:\nhi\n").unwrap();
        std::fs::write(project_dir.join("template.ymmp"), "{}").unwrap();

        let mut p = VideoProject::new("sample", "S", VideoSettings::default());
        p.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![
                Clip::Audio(AudioClip {
                    id: "a1".into(),
                    source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                    start_ms: 0,
                    duration_ms: 10,
                    speaker: "a".into(),
                    extra: BTreeMap::new(),
                }),
                Clip::Audio(AudioClip {
                    id: "a2".into(),
                    source: RelativeAssetPath::new("assets/audio/missing.wav").unwrap(),
                    start_ms: 10,
                    duration_ms: 10,
                    speaker: "a".into(),
                    extra: BTreeMap::new(),
                }),
            ],
        });
        let project_path = project_dir.join("project.vfp.json");
        p.save(&project_path).unwrap();

        let result = create_bundle(
            &project_path,
            None,
            BundleOptions {
                zip: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.dir,
            dir.path().join("generated").join("sample-ymm4-bundle")
        );
        assert!(result.dir.join("project.vfp.json").is_file());
        assert!(result.dir.join("source.md").is_file());
        assert!(result.dir.join("template.ymmp").is_file());
        assert!(result.dir.join("assets/audio/001.wav").is_file());
        assert!(!result.dir.join("assets/audio/missing.wav").exists());
        assert!(result.warnings.iter().any(|w| w.contains("missing.wav")));
        assert!(is_bundle_dir(&result.dir));

        let zip_path = result.zip.unwrap();
        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.contains(&"sample-ymm4-bundle/project.vfp.json".to_string()),
            "{names:?}"
        );
        assert!(names.contains(&"sample-ymm4-bundle/assets/audio/001.wav".to_string()));

        let manifest: BundleManifest = serde_json::from_str(
            &std::fs::read_to_string(result.dir.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.kind, BUNDLE_KIND);
        assert_eq!(manifest.assets, vec!["assets/audio/001.wav"]);
        assert_eq!(manifest.template.as_deref(), Some("template.ymmp"));
    }

    fn minimal_project(project_dir: &Path) -> PathBuf {
        std::fs::create_dir_all(project_dir).unwrap();
        let p = VideoProject::new("sample", "S", VideoSettings::default());
        let project_path = project_dir.join("project.vfp.json");
        p.save(&project_path).unwrap();
        project_path
    }

    #[test]
    fn refuses_to_overwrite_existing_dir_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = minimal_project(&dir.path().join("generated").join("sample"));
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(out_dir.join("keepme.txt"), b"important").unwrap();

        let err = create_bundle(
            &project_path,
            None,
            BundleOptions {
                out_dir: Some(out_dir.clone()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, AppError::UnsafeOverwrite { .. }));
        assert!(out_dir.join("keepme.txt").is_file());
    }

    #[test]
    fn force_refuses_directory_that_is_not_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = minimal_project(&dir.path().join("generated").join("sample"));
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(out_dir.join("keepme.txt"), b"important").unwrap();

        let err = create_bundle(
            &project_path,
            None,
            BundleOptions {
                out_dir: Some(out_dir.clone()),
                force: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, AppError::UnsafeOverwrite { .. }));
        assert!(out_dir.join("keepme.txt").is_file());
    }

    #[test]
    fn force_overwrites_previous_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = minimal_project(&dir.path().join("generated").join("sample"));
        let out_dir = dir.path().join("out");

        create_bundle(
            &project_path,
            None,
            BundleOptions {
                out_dir: Some(out_dir.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(is_bundle_dir(&out_dir));

        // Re-running without --force still refuses.
        assert!(matches!(
            create_bundle(
                &project_path,
                None,
                BundleOptions {
                    out_dir: Some(out_dir.clone()),
                    ..Default::default()
                },
            )
            .unwrap_err(),
            AppError::UnsafeOverwrite { .. }
        ));

        // --force overwrites a directory that is itself a prior bundle.
        let result = create_bundle(
            &project_path,
            None,
            BundleOptions {
                out_dir: Some(out_dir.clone()),
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.dir, out_dir);
    }

    #[test]
    fn force_refuses_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("videoforge.yaml"), "speakers:\n  a: {}\n").unwrap();
        let project_path = minimal_project(&dir.path().join("generated").join("sample"));
        let ws = Workspace::open(dir.path()).unwrap();

        let err = create_bundle(
            &project_path,
            Some(&ws),
            BundleOptions {
                out_dir: Some(dir.path().to_path_buf()),
                force: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, AppError::UnsafeOverwrite { .. }));
    }
}
