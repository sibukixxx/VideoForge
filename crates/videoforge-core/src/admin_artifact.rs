//! Minimal, provider-neutral artifact manifest for manual TechVit Admin intake.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::manifest::Manifest;

pub const ADMIN_ARTIFACT_SCHEMA_VERSION: &str = "techvit-admin-artifact/v1";
pub const ADMIN_ARTIFACT_FILE: &str = "admin-artifact.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub generator_version: String,
    pub project: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdminArtifactManifest {
    pub schema_version: String,
    pub source_system: String,
    pub external_id: String,
    pub idempotency_key: String,
    pub artifact_type: String,
    pub path: String,
    pub title: String,
    pub created_at: String,
    pub source_reference: String,
    pub metadata: ArtifactMetadata,
}

impl AdminArtifactManifest {
    pub fn from_render(
        slug: &str,
        title: &str,
        artifact_path: &str,
        width: u32,
        height: u32,
        manifest: &Manifest,
    ) -> Result<Self, AppError> {
        let value = Self {
            schema_version: ADMIN_ARTIFACT_SCHEMA_VERSION.to_owned(),
            source_system: "videoforge".to_owned(),
            external_id: slug.to_owned(),
            idempotency_key: format!("videoforge:{slug}:{artifact_path}:admin-artifact:v1"),
            artifact_type: "video/mp4".to_owned(),
            path: artifact_path.to_owned(),
            title: title.to_owned(),
            created_at: manifest.generated_at.clone(),
            source_reference: "manifest.json".to_owned(),
            metadata: ArtifactMetadata {
                duration_ms: manifest.duration_ms,
                width,
                height,
                generator_version: manifest.generator_version.clone(),
                project: manifest.project.clone(),
                source: manifest.source.clone(),
            },
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        let required = [
            ("schema_version", self.schema_version.as_str()),
            ("source_system", self.source_system.as_str()),
            ("external_id", self.external_id.as_str()),
            ("idempotency_key", self.idempotency_key.as_str()),
            ("artifact_type", self.artifact_type.as_str()),
            ("path", self.path.as_str()),
            ("title", self.title.as_str()),
            ("created_at", self.created_at.as_str()),
            ("source_reference", self.source_reference.as_str()),
        ];
        if let Some((field, _)) = required.iter().find(|(_, value)| value.trim().is_empty()) {
            return Err(AppError::Other(format!("{field} is required")));
        }
        if self.schema_version != ADMIN_ARTIFACT_SCHEMA_VERSION {
            return Err(AppError::Other("unsupported Admin artifact schema".into()));
        }
        if self.source_system != "videoforge" || self.artifact_type != "video/mp4" {
            return Err(AppError::Other("invalid Admin artifact identity".into()));
        }
        if self.metadata.duration_ms == 0 || self.metadata.width == 0 || self.metadata.height == 0 {
            return Err(AppError::Other(
                "duration_ms, width, and height must be greater than zero".into(),
            ));
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::serialization(ADMIN_ARTIFACT_FILE, e))?;
        std::fs::write(path, format!("{json}\n")).map_err(|e| AppError::write(path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_manifest() -> Manifest {
        Manifest {
            schema_version: 1,
            generator_version: "0.1.0".into(),
            source: "scripts/demo.md".into(),
            project: "project.vfp.json".into(),
            preview: Some("preview.mp4".into()),
            captions: "captions.srt".into(),
            audio: vec![],
            generated_at: "2026-09-17T00:00:00Z".into(),
            platform: "test".into(),
            duration_ms: 42_100,
            dialogues: 2,
            warnings: vec![],
        }
    }

    #[test]
    fn render_manifest_is_stable_and_traceable() {
        let first = AdminArtifactManifest::from_render(
            "demo",
            "Demo",
            "preview.mp4",
            1080,
            1920,
            &source_manifest(),
        )
        .unwrap();
        let second = AdminArtifactManifest::from_render(
            "demo",
            "Demo",
            "preview.mp4",
            1080,
            1920,
            &source_manifest(),
        )
        .unwrap();
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(first.source_system, "videoforge");
        assert_eq!(first.artifact_type, "video/mp4");
        assert_eq!(first.metadata.duration_ms, 42_100);
    }

    #[test]
    fn missing_output_is_rejected() {
        let err = AdminArtifactManifest::from_render(
            "demo",
            "Demo",
            "",
            1080,
            1920,
            &source_manifest(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("path is required"));
    }
}
