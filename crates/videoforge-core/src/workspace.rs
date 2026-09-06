//! Workspace resolver (design §5).

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::AppError;

pub const CONFIG_FILE: &str = "videoforge.yaml";
pub const AGENTS_FILE: &str = "AGENTS.md";
pub const GENERATED_DIR: &str = "generated";
pub const TMP_DIR: &str = ".generated-tmp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Open a workspace whose root contains `videoforge.yaml`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, AppError> {
        let root = root.as_ref();
        if !root.join(CONFIG_FILE).is_file() {
            return Err(AppError::WorkspaceNotFound(root.to_path_buf()));
        }
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        Ok(Self { root })
    }

    /// Walk up from `start` (a file or directory) until `videoforge.yaml` is found.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, AppError> {
        let start = start.as_ref();
        let start_abs = if start.is_absolute() {
            start.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| AppError::Other(format!("cannot read current dir: {e}")))?
                .join(start)
        };
        let mut dir: Option<&Path> = if start_abs.is_dir() {
            Some(start_abs.as_path())
        } else {
            start_abs.parent()
        };
        while let Some(d) = dir {
            if d.join(CONFIG_FILE).is_file() {
                return Self::open(d);
            }
            dir = d.parent();
        }
        Err(AppError::WorkspaceNotFound(start_abs))
    }

    /// Discover from the current directory.
    pub fn discover_cwd() -> Result<Self, AppError> {
        let cwd = std::env::current_dir()
            .map_err(|e| AppError::Other(format!("cannot read current dir: {e}")))?;
        Self::discover(cwd)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    pub fn scripts_dir(&self) -> PathBuf {
        self.root.join("scripts")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.root.join("templates")
    }

    pub fn generated_dir(&self) -> PathBuf {
        self.root.join(GENERATED_DIR)
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join(TMP_DIR)
    }

    pub fn load_config(&self) -> Result<Config, AppError> {
        Config::load(&self.config_path())
    }

    /// Resolve a workspace-relative (forward-slash) path to a native path.
    /// Absolute inputs are returned unchanged.
    pub fn resolve(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        let mut out = self.root.clone();
        for part in rel.split(['/', '\\']) {
            if !part.is_empty() && part != "." {
                out.push(part);
            }
        }
        out
    }

    /// Workspace-relative display path with forward slashes. Falls back to the
    /// absolute path when `path` is outside the workspace.
    pub fn relative(&self, path: &Path) -> String {
        let abs = path
            .canonicalize()
            .unwrap_or_else(|_| self.absolutize(path));
        match abs.strip_prefix(&self.root) {
            Ok(rel) => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => abs.display().to_string(),
        }
    }

    fn absolutize(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| self.root.join(path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), "speakers:\n  a: {}\n").unwrap();
        let nested = dir.path().join("scripts").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        let script = nested.join("x.md");
        std::fs::write(&script, "a:\nhi\n").unwrap();

        let ws = Workspace::discover(&script).unwrap();
        assert_eq!(ws.root(), dir.path().canonicalize().unwrap());
        assert_eq!(ws.relative(&script), "scripts/deep/x.md");
        assert_eq!(
            ws.resolve("scripts/deep/x.md"),
            ws.root().join("scripts/deep/x.md")
        );
        assert!(ws.load_config().is_ok());
    }

    #[test]
    fn missing_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let err = Workspace::open(dir.path()).unwrap_err();
        assert!(matches!(err, AppError::WorkspaceNotFound(_)));
    }
}
