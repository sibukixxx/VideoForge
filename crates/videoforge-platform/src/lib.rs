//! Platform adapter. Everything OS-specific that the core needs lives behind
//! the [`Platform`] trait so that `videoforge-core` never touches OS APIs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("no application data directory available on this platform")]
    NoAppDataDir,
    #[error("failed to launch `{program}`: {source}")]
    Launch {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not available on this platform")]
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Windows,
    MacOs,
    Linux,
    Other,
}

impl PlatformKind {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
            Self::Other => "other",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
            Self::Other => "Unknown OS",
        }
    }
}

/// `macos-arm64`, `windows-x86_64`, ... used in manifests and doctor output.
pub fn platform_label() -> String {
    format!("{}-{}", PlatformKind::current().name(), arch_label())
}

pub fn arch_label() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    }
}

pub trait Platform: Send + Sync {
    fn kind(&self) -> PlatformKind;

    /// Show the file (or directory) in Explorer / Finder / the desktop file manager.
    fn reveal_in_file_manager(&self, path: &Path) -> Result<(), PlatformError>;

    /// Open a file with its default application.
    fn open_file(&self, path: &Path) -> Result<(), PlatformError>;

    /// Per-user application data directory for VideoForge.
    fn app_data_dir(&self) -> Result<PathBuf, PlatformError>;

    /// Per-user cache directory for VideoForge.
    ///
    /// * Windows: `%LOCALAPPDATA%\VideoForge\cache`
    /// * macOS: `~/Library/Caches/VideoForge`
    /// * Linux: `$XDG_CACHE_HOME/VideoForge`
    fn cache_dir(&self) -> Result<PathBuf, PlatformError>;

    /// Locate a YukkuriMovieMaker4 executable, if this platform can run it.
    fn find_ymm4(&self) -> Option<PathBuf>;

    /// Launch YMM4 with the given project. Only meaningful on Windows.
    fn open_in_ymm4(&self, project: &Path) -> Result<(), PlatformError>;
}

pub fn current_platform() -> Box<dyn Platform> {
    Box::new(NativePlatform)
}

/// Adapter for the OS this binary was compiled for.
pub struct NativePlatform;

fn spawn(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), PlatformError> {
    Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|source| PlatformError::Launch {
            program: program.to_string(),
            source,
        })
}

impl Platform for NativePlatform {
    fn kind(&self) -> PlatformKind {
        PlatformKind::current()
    }

    fn reveal_in_file_manager(&self, path: &Path) -> Result<(), PlatformError> {
        match self.kind() {
            PlatformKind::Windows => {
                let arg = format!("/select,{}", path.display());
                spawn("explorer", &[arg.as_ref()])
            }
            PlatformKind::MacOs => spawn("open", &["-R".as_ref(), path.as_os_str()]),
            _ => {
                let target = if path.is_dir() {
                    path.to_path_buf()
                } else {
                    path.parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| path.to_path_buf())
                };
                spawn("xdg-open", &[target.as_os_str()])
            }
        }
    }

    fn open_file(&self, path: &Path) -> Result<(), PlatformError> {
        match self.kind() {
            PlatformKind::Windows => spawn(
                "cmd",
                &[
                    "/C".as_ref(),
                    "start".as_ref(),
                    "".as_ref(),
                    path.as_os_str(),
                ],
            ),
            PlatformKind::MacOs => spawn("open", &[path.as_os_str()]),
            _ => spawn("xdg-open", &[path.as_os_str()]),
        }
    }

    fn app_data_dir(&self) -> Result<PathBuf, PlatformError> {
        if let Some(dir) = std::env::var_os("VIDEOFORGE_DATA_DIR") {
            return Ok(PathBuf::from(dir));
        }
        dirs::data_local_dir()
            .map(|d| d.join("VideoForge"))
            .ok_or(PlatformError::NoAppDataDir)
    }

    fn cache_dir(&self) -> Result<PathBuf, PlatformError> {
        if let Some(dir) = std::env::var_os("VIDEOFORGE_CACHE_DIR") {
            return Ok(PathBuf::from(dir));
        }
        let base = dirs::cache_dir().ok_or(PlatformError::NoAppDataDir)?;
        Ok(match self.kind() {
            // dirs::cache_dir() is %LOCALAPPDATA% on Windows
            PlatformKind::Windows => base.join("VideoForge").join("cache"),
            _ => base.join("VideoForge"),
        })
    }

    fn find_ymm4(&self) -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("VIDEOFORGE_YMM4_PATH") {
            let p = PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
        }
        if self.kind() != PlatformKind::Windows {
            return None;
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        for var in [
            "LOCALAPPDATA",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "USERPROFILE",
        ] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                candidates.push(
                    base.join("YukkuriMovieMaker4")
                        .join("YukkuriMovieMaker.exe"),
                );
                candidates.push(
                    base.join("Programs")
                        .join("YukkuriMovieMaker4")
                        .join("YukkuriMovieMaker.exe"),
                );
                candidates.push(
                    base.join("Desktop")
                        .join("YukkuriMovieMaker4")
                        .join("YukkuriMovieMaker.exe"),
                );
            }
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    fn open_in_ymm4(&self, project: &Path) -> Result<(), PlatformError> {
        let exe = self
            .find_ymm4()
            .ok_or(PlatformError::Unsupported("YukkuriMovieMaker4"))?;
        Command::new(&exe)
            .arg(project)
            .spawn()
            .map(|_| ())
            .map_err(|source| PlatformError::Launch {
                program: exe.display().to_string(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_has_os_and_arch() {
        let label = platform_label();
        assert!(label.contains('-'));
        assert!(label.starts_with(PlatformKind::current().name()));
    }

    #[test]
    fn cache_dir_env_override() {
        std::env::set_var("VIDEOFORGE_CACHE_DIR", "/tmp/vf-cache-test");
        let dir = NativePlatform.cache_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/vf-cache-test"));
        std::env::remove_var("VIDEOFORGE_CACHE_DIR");
        let dir = NativePlatform.cache_dir().unwrap();
        assert!(dir.ends_with("VideoForge") || dir.ends_with("cache"));
    }

    #[test]
    fn ymm4_not_found_off_windows() {
        if PlatformKind::current() != PlatformKind::Windows {
            std::env::remove_var("VIDEOFORGE_YMM4_PATH");
            assert!(NativePlatform.find_ymm4().is_none());
        }
    }
}
