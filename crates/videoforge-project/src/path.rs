use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A path to an asset, relative to the project root (the directory containing
/// `project.vfp.json`). Always stored with forward slashes.
///
/// ```
/// use videoforge_project::RelativeAssetPath;
/// assert!(RelativeAssetPath::new("assets/audio/001.wav").is_ok());
/// assert!(RelativeAssetPath::new(r"C:\data\001.wav").is_err());
/// assert!(RelativeAssetPath::new("/Users/foo/data/001.wav").is_err());
/// assert!(RelativeAssetPath::new("../outside.wav").is_err());
/// ```
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelativeAssetPath(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("asset path must not be empty")]
    Empty,
    #[error("asset path must be relative to the project root, got absolute path: {0}")]
    Absolute(String),
    #[error("asset path must not contain backslashes (use '/'): {0}")]
    Backslash(String),
    #[error("asset path must not escape the project root ('..'): {0}")]
    ParentTraversal(String),
    #[error("asset path contains an invalid component: {0}")]
    InvalidComponent(String),
}

impl RelativeAssetPath {
    /// Validate and normalize a relative asset path.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, PathError> {
        let raw = raw.as_ref();
        if raw.trim().is_empty() {
            return Err(PathError::Empty);
        }
        if raw.contains('\\') {
            return Err(PathError::Backslash(raw.to_string()));
        }
        if is_absolute_like(raw) {
            return Err(PathError::Absolute(raw.to_string()));
        }

        let mut parts: Vec<&str> = Vec::new();
        for segment in raw.split('/') {
            match segment {
                "" | "." => continue,
                ".." => return Err(PathError::ParentTraversal(raw.to_string())),
                s if s.contains(':') => return Err(PathError::InvalidComponent(raw.to_string())),
                s => parts.push(s),
            }
        }
        if parts.is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(parts.join("/")))
    }

    /// Build a relative asset path from a native path that is already known to
    /// be relative (for example produced by `Path::strip_prefix`).
    pub fn from_native(path: &Path) -> Result<Self, PathError> {
        let mut parts = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(PathError::ParentTraversal(path.display().to_string()))
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute(path.display().to_string()))
                }
            }
        }
        Self::new(parts.join("/"))
    }

    /// The canonical forward-slash string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolve to a native path under `root`.
    pub fn resolve(&self, root: &Path) -> PathBuf {
        let mut out = root.to_path_buf();
        for part in self.0.split('/') {
            out.push(part);
        }
        out
    }

    /// Last path segment.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Join a child segment (validated).
    pub fn join(&self, child: &str) -> Result<Self, PathError> {
        Self::new(format!("{}/{}", self.0, child))
    }
}

fn is_absolute_like(raw: &str) -> bool {
    if raw.starts_with('/') {
        return true;
    }
    // Windows drive letter: "C:" or "C:/"
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    // UNC in forward-slash form
    raw.starts_with("//")
}

impl TryFrom<String> for RelativeAssetPath {
    type Error = PathError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for RelativeAssetPath {
    type Error = PathError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RelativeAssetPath> for String {
    fn from(value: RelativeAssetPath) -> Self {
        value.0
    }
}

impl fmt::Display for RelativeAssetPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for RelativeAssetPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RelativeAssetPath({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_paths() {
        let p = RelativeAssetPath::new("assets/audio/001.wav").unwrap();
        assert_eq!(p.as_str(), "assets/audio/001.wav");
        assert_eq!(p.file_name(), "001.wav");
    }

    #[test]
    fn normalizes_dot_segments_and_duplicate_slashes() {
        let p = RelativeAssetPath::new("./assets//audio/./001.wav").unwrap();
        assert_eq!(p.as_str(), "assets/audio/001.wav");
    }

    #[test]
    fn rejects_windows_absolute_paths() {
        assert_eq!(
            RelativeAssetPath::new(r"C:\data\001.wav"),
            Err(PathError::Backslash(r"C:\data\001.wav".into()))
        );
        assert_eq!(
            RelativeAssetPath::new("C:/data/001.wav"),
            Err(PathError::Absolute("C:/data/001.wav".into()))
        );
        assert!(matches!(
            RelativeAssetPath::new("//server/share/a.wav"),
            Err(PathError::Absolute(_))
        ));
    }

    #[test]
    fn rejects_posix_absolute_paths() {
        assert_eq!(
            RelativeAssetPath::new("/Users/foo/data/001.wav"),
            Err(PathError::Absolute("/Users/foo/data/001.wav".into()))
        );
    }

    #[test]
    fn rejects_parent_traversal_and_empty() {
        assert!(matches!(
            RelativeAssetPath::new("../x.wav"),
            Err(PathError::ParentTraversal(_))
        ));
        assert!(matches!(
            RelativeAssetPath::new("a/../../x.wav"),
            Err(PathError::ParentTraversal(_))
        ));
        assert_eq!(RelativeAssetPath::new(""), Err(PathError::Empty));
        assert_eq!(RelativeAssetPath::new("./"), Err(PathError::Empty));
    }

    #[test]
    fn serde_roundtrip_and_rejects_absolute_on_deserialize() {
        let p = RelativeAssetPath::new("assets/a.wav").unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"assets/a.wav\"");
        let back: RelativeAssetPath = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);

        let err = serde_json::from_str::<RelativeAssetPath>("\"/abs/a.wav\"").unwrap_err();
        assert!(err.to_string().contains("absolute"));
        let err = serde_json::from_str::<RelativeAssetPath>("\"C:\\\\abs\\\\a.wav\"").unwrap_err();
        assert!(err.to_string().contains("backslash"));
    }

    #[test]
    fn resolves_under_root() {
        let p = RelativeAssetPath::new("assets/audio/001.wav").unwrap();
        let root = Path::new("root");
        let resolved = p.resolve(root);
        assert_eq!(
            resolved,
            Path::new("root")
                .join("assets")
                .join("audio")
                .join("001.wav")
        );
    }

    #[test]
    fn from_native_relative() {
        let p =
            RelativeAssetPath::from_native(Path::new("assets").join("x.png").as_path()).unwrap();
        assert_eq!(p.as_str(), "assets/x.png");
    }
}
