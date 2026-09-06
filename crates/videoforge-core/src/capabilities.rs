//! Capability detection result (design §25). Computed in Rust; the GUI never
//! branches on the OS name alone.

use serde::{Deserialize, Serialize};
use videoforge_platform::PlatformKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub platform: PlatformKind,
    /// e.g. `macos-arm64`
    pub platform_label: String,
    pub voicevox_available: bool,
    pub voicevox_endpoint: String,
    pub ffmpeg_available: bool,
    pub can_export_ymm4: bool,
    pub can_open_ymm4: bool,
}
