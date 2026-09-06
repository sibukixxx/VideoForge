use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::path::RelativeAssetPath;

/// Current IR schema version. Bump when a change is not backwards compatible.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("failed to read project file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write project file {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid project JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported project schema_version {found} (this build supports up to {supported})")]
    UnsupportedSchema { found: u64, supported: u32 },
    #[error("project is missing schema_version")]
    MissingSchemaVersion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoProject {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub video: VideoSettings,
    pub source: SourceInfo,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Unknown top-level fields are preserved across load/save.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
        }
    }
}

/// Where the project came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceInfo {
    /// Workspace-relative path of the script that produced this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// Template name declared in the script front matter, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Version of the generator that produced this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Caption,
    Character,
    Image,
    Background,
    SoundEffect,
}

/// A clip on a track. Tagged by `type` so exporters can dispatch on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Clip {
    Audio(AudioClip),
    Caption(CaptionClip),
    Background(BackgroundClip),
}

impl Clip {
    pub fn id(&self) -> &str {
        match self {
            Clip::Audio(c) => &c.id,
            Clip::Caption(c) => &c.id,
            Clip::Background(c) => &c.id,
        }
    }

    pub fn start_ms(&self) -> u64 {
        match self {
            Clip::Audio(c) => c.start_ms,
            Clip::Caption(c) => c.start_ms,
            Clip::Background(c) => c.start_ms,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        match self {
            Clip::Audio(c) => c.duration_ms,
            Clip::Caption(c) => c.duration_ms,
            Clip::Background(c) => c.duration_ms,
        }
    }

    pub fn end_ms(&self) -> u64 {
        self.start_ms() + self.duration_ms()
    }

    /// Asset referenced by this clip, if any.
    pub fn asset(&self) -> Option<&RelativeAssetPath> {
        match self {
            Clip::Audio(c) => Some(&c.source),
            Clip::Caption(_) => None,
            Clip::Background(c) => Some(&c.source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Canonical speaker key (e.g. `reimu`).
    pub speaker: String,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionClip {
    pub id: String,
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Canonical speaker key (e.g. `reimu`).
    pub speaker: String,
    /// Display name as written in the script (e.g. `霊夢`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_display: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl VideoProject {
    pub fn new(id: impl Into<String>, title: impl Into<String>, video: VideoSettings) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: id.into(),
            title: title.into(),
            video,
            source: SourceInfo::default(),
            tracks: Vec::new(),
            extra: BTreeMap::new(),
        }
    }

    pub fn tracks_of(&self, kind: TrackKind) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(move |t| t.kind == kind)
    }

    pub fn audio_clips(&self) -> Vec<&AudioClip> {
        self.tracks_of(TrackKind::Audio)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Audio(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn caption_clips(&self) -> Vec<&CaptionClip> {
        self.tracks_of(TrackKind::Caption)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Caption(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn background_clips(&self) -> Vec<&BackgroundClip> {
        self.tracks_of(TrackKind::Background)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Background(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    /// All clips across all tracks.
    pub fn clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|t| t.clips.iter())
    }

    /// Total timeline length: the latest clip end.
    pub fn total_duration_ms(&self) -> u64 {
        self.clips().map(Clip::end_ms).max().unwrap_or(0)
    }

    /// Distinct asset paths referenced by the project, sorted.
    pub fn referenced_assets(&self) -> Vec<RelativeAssetPath> {
        let mut assets: Vec<RelativeAssetPath> =
            self.clips().filter_map(Clip::asset).cloned().collect();
        assets.sort();
        assets.dedup();
        assets
    }

    /// Serialize as pretty JSON.
    pub fn to_json(&self) -> Result<String, ProjectError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parse from JSON, applying schema migration when needed.
    pub fn from_json(json: &str) -> Result<Self, ProjectError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        Self::from_value(value)
    }

    /// Migration entry point: accept any supported `schema_version` and lift it
    /// to the current one.
    pub fn from_value(value: serde_json::Value) -> Result<Self, ProjectError> {
        let found = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .ok_or(ProjectError::MissingSchemaVersion)?;
        if found > SCHEMA_VERSION as u64 {
            return Err(ProjectError::UnsupportedSchema {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        // Future: match on `found` and apply migrations step by step.
        let project: VideoProject = serde_json::from_value(value)?;
        Ok(project)
    }

    pub fn load(path: &Path) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path).map_err(|source| ProjectError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_json(&text)
    }

    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        let json = self.to_json()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ProjectError::Write {
                path: path.display().to_string(),
                source,
            })?;
        }
        std::fs::write(path, json).map_err(|source| ProjectError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VideoProject {
        let mut p = VideoProject::new("sample", "Sample", VideoSettings::default());
        p.source.script = Some("scripts/sample.md".into());
        p.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![Clip::Audio(AudioClip {
                id: "audio-001".into(),
                source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                start_ms: 0,
                duration_ms: 3410,
                speaker: "reimu".into(),
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "caption".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "caption-001".into(),
                text: "こんにちは".into(),
                start_ms: 0,
                duration_ms: 3410,
                speaker: "reimu".into(),
                speaker_display: Some("霊夢".into()),
                extra: BTreeMap::new(),
            })],
        });
        p
    }

    #[test]
    fn roundtrip_json() {
        let p = sample();
        let json = p.to_json().unwrap();
        assert!(json.contains("\"schema_version\": 1"));
        assert!(json.contains("\"type\": \"audio\""));
        let back = VideoProject::from_json(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.total_duration_ms(), 3410);
        assert_eq!(back.audio_clips().len(), 1);
        assert_eq!(back.caption_clips().len(), 1);
        assert_eq!(back.referenced_assets().len(), 1);
    }

    #[test]
    fn preserves_unknown_fields() {
        let json = r#"{
            "schema_version": 1,
            "id": "x", "title": "X",
            "video": {"width": 1280, "height": 720, "fps": 24},
            "source": {},
            "tracks": [],
            "future_field": {"a": 1}
        }"#;
        let p = VideoProject::from_json(json).unwrap();
        assert_eq!(p.video.fps, 24);
        let out = p.to_json().unwrap();
        assert!(out.contains("future_field"));
    }

    #[test]
    fn rejects_newer_schema() {
        let json = r#"{"schema_version": 99, "id": "x", "title": "X",
            "video": {"width": 1, "height": 1, "fps": 1}, "source": {}, "tracks": []}"#;
        let err = VideoProject::from_json(json).unwrap_err();
        assert!(matches!(
            err,
            ProjectError::UnsupportedSchema { found: 99, .. }
        ));
    }

    #[test]
    fn rejects_absolute_asset_in_json() {
        let json = r#"{"schema_version": 1, "id": "x", "title": "X",
            "video": {"width": 1, "height": 1, "fps": 1}, "source": {},
            "tracks": [{"id": "a", "kind": "audio", "clips": [
              {"type": "audio", "id": "c", "source": "C:/x/a.wav",
               "start_ms": 0, "duration_ms": 1, "speaker": "s"}]}]}"#;
        assert!(VideoProject::from_json(json).is_err());
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("project.vfp.json");
        let p = sample();
        p.save(&path).unwrap();
        let back = VideoProject::load(&path).unwrap();
        assert_eq!(back, p);
    }
}
