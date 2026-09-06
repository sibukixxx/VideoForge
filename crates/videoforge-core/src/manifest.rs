//! `manifest.json` written next to every generated project (design §35).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub generator_version: String,
    /// Workspace-relative script path.
    pub source: String,
    /// Project file, relative to the manifest.
    pub project: String,
    pub preview: Option<String>,
    pub captions: String,
    #[serde(default)]
    pub audio: Vec<String>,
    pub generated_at: String,
    pub platform: String,
    pub duration_ms: u64,
    pub dialogues: usize,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Manifest {
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::serialization("manifest.json", e))?;
        std::fs::write(path, json).map_err(|e| AppError::write(path, e))
    }

    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = std::fs::read_to_string(path).map_err(|e| AppError::read(path, e))?;
        serde_json::from_str(&text).map_err(|e| AppError::serialization("manifest.json", e))
    }
}
