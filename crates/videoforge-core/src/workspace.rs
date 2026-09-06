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

    /// Resolve a workspace-relative (forward-slash or native) path to a
    /// native path guaranteed to stay inside the workspace root.
    ///
    /// Rejects absolute inputs, `..` segments, and drive-letter-like segments
    /// (a Windows path-traversal gotcha: `C:foo` is drive-relative, not
    /// absolute). Also rejects paths that would escape the workspace root via
    /// a symlink in an already-existing ancestor. Use
    /// [`Workspace::resolve_allow_absolute`] for settings that explicitly
    /// document absolute-path support.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, AppError> {
        let candidate = self.join_relative(rel)?;
        self.ensure_within_root(&candidate, rel)?;
        Ok(candidate)
    }

    /// Like [`Workspace::resolve`], but an absolute `rel` is returned
    /// unchanged instead of being rejected. Only use this for settings that
    /// explicitly document absolute-path support (e.g. `preview.font`);
    /// prefer [`Workspace::resolve`] everywhere else.
    pub fn resolve_allow_absolute(&self, rel: &str) -> Result<PathBuf, AppError> {
        let p = Path::new(rel);
        if p.is_absolute() {
            return Ok(p.to_path_buf());
        }
        self.resolve(rel)
    }

    fn join_relative(&self, rel: &str) -> Result<PathBuf, AppError> {
        let invalid = |reason: &str| AppError::InvalidWorkspacePath {
            path: rel.to_string(),
            reason: reason.to_string(),
        };
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err(invalid("absolute paths are not allowed here"));
        }
        let mut out = self.root.clone();
        for part in rel.split(['/', '\\']) {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                return Err(invalid("`..` is not allowed in workspace-relative paths"));
            }
            if part.contains(':') {
                return Err(invalid(
                    "`:` is not allowed in workspace-relative path segments",
                ));
            }
            out.push(part);
        }
        Ok(out)
    }

    /// Canonicalizes the longest existing ancestor of `candidate` (which may
    /// not exist yet, e.g. a not-yet-generated output path) and checks it is
    /// still inside the workspace root, catching escapes via symlinks.
    fn ensure_within_root(&self, candidate: &Path, original: &str) -> Result<(), AppError> {
        let mut check = candidate.to_path_buf();
        let mut trailing: Vec<std::ffi::OsString> = Vec::new();
        loop {
            match check.canonicalize() {
                Ok(canon) => {
                    let mut full = canon;
                    for part in trailing.iter().rev() {
                        full.push(part);
                    }
                    return if full.starts_with(&self.root) {
                        Ok(())
                    } else {
                        Err(AppError::InvalidWorkspacePath {
                            path: original.to_string(),
                            reason: "path escapes the workspace root".into(),
                        })
                    };
                }
                Err(_) => match check.file_name().map(|s| s.to_os_string()) {
                    Some(name) => {
                        trailing.push(name);
                        check = check
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_else(|| self.root.clone());
                    }
                    None => return Ok(()),
                },
            }
        }
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
            ws.resolve("scripts/deep/x.md").unwrap(),
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

    fn open_ws() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(CONFIG_FILE), "speakers:\n  a: {}\n").unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn rejects_parent_traversal() {
        let (_dir, ws) = open_ws();
        assert!(matches!(
            ws.resolve("../foo").unwrap_err(),
            AppError::InvalidWorkspacePath { .. }
        ));
        assert!(matches!(
            ws.resolve("../../foo").unwrap_err(),
            AppError::InvalidWorkspacePath { .. }
        ));
        assert!(matches!(
            ws.resolve("assets/../../foo").unwrap_err(),
            AppError::InvalidWorkspacePath { .. }
        ));
    }

    #[test]
    fn rejects_absolute_paths() {
        let (_dir, ws) = open_ws();
        assert!(matches!(
            ws.resolve("/etc/passwd").unwrap_err(),
            AppError::InvalidWorkspacePath { .. }
        ));
    }

    #[test]
    fn rejects_drive_letter_like_segments() {
        let (_dir, ws) = open_ws();
        assert!(matches!(
            ws.resolve("C:evil.txt").unwrap_err(),
            AppError::InvalidWorkspacePath { .. }
        ));
    }

    #[test]
    fn resolves_normal_relative_paths() {
        let (_dir, ws) = open_ws();
        assert_eq!(
            ws.resolve("assets/audio/001.wav").unwrap(),
            ws.root().join("assets").join("audio").join("001.wav")
        );
    }

    #[test]
    fn resolve_allow_absolute_permits_absolute_only_there() {
        let (_dir, ws) = open_ws();
        assert_eq!(
            ws.resolve_allow_absolute("/usr/share/fonts/x.ttf").unwrap(),
            PathBuf::from("/usr/share/fonts/x.ttf")
        );
        assert_eq!(
            ws.resolve_allow_absolute("assets/fonts/x.ttf").unwrap(),
            ws.root().join("assets").join("fonts").join("x.ttf")
        );
        assert!(ws.resolve_allow_absolute("../escape.ttf").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (dir, ws) = open_ws();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        let err = ws.resolve("escape/x.txt").unwrap_err();
        assert!(matches!(err, AppError::InvalidWorkspacePath { .. }));
    }
}
