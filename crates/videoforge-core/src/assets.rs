//! Asset registry (P0-2): identifies every file a generated project depends
//! on, checks it exists, and hashes it — the minimum needed for
//! reproducibility and provenance tracking, deliberately not a full digital
//! asset management system (design: "P0で巨大なDAMを作らない").
//!
//! Two kinds of entries:
//!
//! * One per referenced [`RelativeAssetPath`] on the project's own clips
//!   (audio, background, image, the legacy `@character` stand-in, bgm, sound
//!   effect, and a character performance's lip-sync curve) — paths relative
//!   to the project directory, matching [`VideoProject::referenced_assets`].
//! * One per character in the linked character manifest (design §5), whose
//!   files (a Live2D `model3.json`, or a `png_lipsync` model's three
//!   sprites) live *outside* the project directory and are resolved against
//!   the character manifest's own directory instead (same rule as
//!   `CharacterModel::resolve_path`).
//!
//! Written to `generated/<slug>/asset-registry.json` alongside
//! `manifest.json`; read back by `videoforge assets`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use videoforge_character::CharacterManifest;
use videoforge_project::{Clip, VideoProject};

use crate::error::AppError;

pub const ASSET_REGISTRY_SCHEMA_VERSION: u32 = 1;

/// What kind of thing an asset is. Kept as a string (not a Rust enum
/// re-exported to JSON as a closed set) would also work, but a closed set
/// here mirrors `videoforge_project::TrackKind` — an asset registry entry
/// exists *because* a clip or a character manifest referenced it, and those
/// origins are already a fixed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Character,
    Image,
    Video,
    Audio,
    Bgm,
    SoundEffect,
    Font,
    /// Output of the pipeline itself (e.g. a lip-sync curve JSON), not
    /// something the user supplied.
    Generated,
}

/// Where an asset came from. Free-form (like `Presentation::role`) rather
/// than a closed enum: P0 only ever writes `"user-provided"` or
/// `"generated"`, but this is metadata a human may reasonably want to
/// annotate by hand later without a schema change.
pub const PROVENANCE_USER_PROVIDED: &str = "user-provided";
pub const PROVENANCE_GENERATED: &str = "generated";

/// License clearance status. `"unknown"` is the deliberate default for
/// anything user-provided (matches `docs/character-licensing.md`'s own
/// stance: never assume a character/model asset is clear to use) — P0 does
/// not attempt to infer this.
pub const LICENSE_UNKNOWN: &str = "unknown";
pub const LICENSE_GENERATED: &str = "n/a (generated output)";

/// One named file within an asset (e.g. `closed`/`half`/`open` for a
/// `png_lipsync` character, or a single `default` entry for anything with
/// just one file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetFile {
    pub role: String,
    pub path: String,
    pub exists: bool,
    /// `sha256:<hex>`, present only when the file exists and was readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRecord {
    /// Stable within one registry: `<kind>.<clip-id>` for a clip-derived
    /// entry, `character.<character-id>` for a character manifest entry.
    pub id: String,
    pub kind: AssetKind,
    pub files: Vec<AssetFile>,
    pub provenance: String,
    pub license_status: String,
}

impl AssetRecord {
    pub fn exists(&self) -> bool {
        !self.files.is_empty() && self.files.iter().all(|f| f.exists)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRegistry {
    pub schema_version: u32,
    pub assets: Vec<AssetRecord>,
}

impl AssetRegistry {
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::serialization("asset-registry.json", e))?;
        std::fs::write(path, json).map_err(|e| AppError::write(path, e))
    }

    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = std::fs::read_to_string(path).map_err(|e| AppError::read(path, e))?;
        serde_json::from_str(&text).map_err(|e| AppError::serialization("asset-registry.json", e))
    }

    /// Assets that are missing, or have at least one missing file.
    pub fn missing(&self) -> Vec<&AssetRecord> {
        self.assets.iter().filter(|a| !a.exists()).collect()
    }
}

fn kind_for_clip(clip: &Clip) -> Option<AssetKind> {
    match clip {
        Clip::Audio(_) => Some(AssetKind::Audio),
        Clip::Caption(_) => None,
        Clip::Background(_) => Some(AssetKind::Image),
        Clip::Image(_) => Some(AssetKind::Image),
        Clip::Character(_) => Some(AssetKind::Image),
        Clip::Bgm(_) => Some(AssetKind::Bgm),
        Clip::SoundEffect(_) => Some(AssetKind::SoundEffect),
        Clip::CharacterPerformance(_) => Some(AssetKind::Generated),
    }
}

fn hash_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("sha256:{:x}", hasher.finalize()))
}

/// Resolve `value` against `base` unless it is already absolute — the same
/// rule `CharacterModel::resolve_path` uses, duplicated here in miniature
/// rather than exposing that crate's private helper.
fn resolve_relative_to(base: &Path, value: &str) -> std::path::PathBuf {
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn file_entry(role: &str, rel: &str, resolved: &Path) -> AssetFile {
    let exists = resolved.is_file();
    AssetFile {
        role: role.to_string(),
        path: rel.to_string(),
        exists,
        hash: if exists { hash_file(resolved) } else { None },
    }
}

/// Build the registry for a generated project. `project_dir` resolves every
/// clip asset path (same rule as `RelativeAssetPath::resolve`);
/// `character_manifest`, when given, adds one entry per character that a
/// `character_performance` clip actually references (not every character
/// the manifest declares — an unused character has nothing generated for
/// it).
pub fn build_registry(
    project: &VideoProject,
    project_dir: &Path,
    character_manifest: Option<(&CharacterManifest, &Path)>,
) -> AssetRegistry {
    let mut assets = Vec::new();

    for clip in project.clips() {
        let Some(kind) = kind_for_clip(clip) else {
            continue;
        };
        let Some(asset) = clip.asset() else { continue };
        let rel = asset.as_str();
        let resolved = asset.resolve(project_dir);
        let provenance = if kind == AssetKind::Generated {
            PROVENANCE_GENERATED
        } else {
            PROVENANCE_USER_PROVIDED
        };
        let license_status = if kind == AssetKind::Generated {
            LICENSE_GENERATED
        } else {
            LICENSE_UNKNOWN
        };
        assets.push(AssetRecord {
            id: format!("{}.{}", kind_label(kind), clip.id()),
            kind,
            files: vec![file_entry("default", rel, &resolved)],
            provenance: provenance.to_string(),
            license_status: license_status.to_string(),
        });
    }

    if let Some((manifest, manifest_dir)) = character_manifest {
        let referenced: std::collections::BTreeSet<&str> = project
            .character_performance_clips()
            .iter()
            .map(|c| c.character.as_str())
            .collect();
        for character in &manifest.characters {
            if !referenced.contains(character.id.as_str()) {
                continue;
            }
            let Some(model) = &character.model else {
                continue;
            };
            let files = if model.is_png_lipsync() {
                [
                    ("closed", &model.closed),
                    ("half", &model.half),
                    ("open", &model.open),
                ]
                .into_iter()
                .filter_map(|(role, path)| {
                    let path = path.as_deref()?;
                    let resolved = resolve_relative_to(manifest_dir, path);
                    Some(file_entry(role, path, &resolved))
                })
                .collect()
            } else {
                match &model.path {
                    Some(path) => {
                        let resolved = model.resolve_path(manifest_dir);
                        vec![file_entry("model", path, &resolved)]
                    }
                    None => Vec::new(),
                }
            };
            if files.is_empty() {
                continue;
            }
            assets.push(AssetRecord {
                id: format!("character.{}", character.id),
                kind: AssetKind::Character,
                files,
                provenance: PROVENANCE_USER_PROVIDED.to_string(),
                license_status: LICENSE_UNKNOWN.to_string(),
            });
        }
    }

    AssetRegistry {
        schema_version: ASSET_REGISTRY_SCHEMA_VERSION,
        assets,
    }
}

fn kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Character => "character",
        AssetKind::Image => "image",
        AssetKind::Video => "video",
        AssetKind::Audio => "audio",
        AssetKind::Bgm => "bgm",
        AssetKind::SoundEffect => "sound_effect",
        AssetKind::Font => "font",
        AssetKind::Generated => "generated",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use videoforge_project::{
        AudioClip, CaptionClip, CharacterPerformanceClip, RelativeAssetPath, Track, TrackKind,
        Transform, VideoSettings,
    };

    fn sample_project(dir: &Path) -> VideoProject {
        std::fs::create_dir_all(dir.join("assets/audio")).unwrap();
        std::fs::write(dir.join("assets/audio/001.wav"), b"fake wav").unwrap();

        let mut p = VideoProject::new("t", "T", VideoSettings::default());
        p.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![Clip::Audio(AudioClip {
                id: "audio-001".into(),
                source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                start_ms: 0,
                duration_ms: 1000,
                speaker: "a".into(),
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "caption".into(),
            kind: TrackKind::Caption,
            clips: vec![Clip::Caption(CaptionClip {
                id: "caption-001".into(),
                text: "hi".into(),
                start_ms: 0,
                duration_ms: 1000,
                speaker: "a".into(),
                speaker_display: None,
                extra: BTreeMap::new(),
            })],
        });
        p
    }

    #[test]
    fn registers_existing_and_missing_assets() {
        let dir = tempfile::tempdir().unwrap();
        let project = sample_project(dir.path());
        let registry = build_registry(&project, dir.path(), None);

        // Caption clips have no asset(); only the audio clip registers.
        assert_eq!(registry.assets.len(), 1);
        let a = &registry.assets[0];
        assert_eq!(a.id, "audio.audio-001");
        assert_eq!(a.kind, AssetKind::Audio);
        assert!(a.exists());
        assert!(a.files[0].hash.as_deref().unwrap().starts_with("sha256:"));
        assert!(registry.missing().is_empty());
    }

    #[test]
    fn missing_file_is_reported_without_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = sample_project(dir.path());
        std::fs::remove_file(dir.path().join("assets/audio/001.wav")).unwrap();
        // add a second clip referencing a file that never existed
        project.tracks[0].clips.push(Clip::Audio(AudioClip {
            id: "audio-002".into(),
            source: RelativeAssetPath::new("assets/audio/002.wav").unwrap(),
            start_ms: 1000,
            duration_ms: 500,
            speaker: "a".into(),
            extra: BTreeMap::new(),
        }));

        let registry = build_registry(&project, dir.path(), None);
        assert_eq!(registry.missing().len(), 2);
        for a in registry.missing() {
            assert!(a.files[0].hash.is_none());
        }
    }

    #[test]
    fn character_performance_asset_is_the_lipsync_curve_not_the_sprites() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = sample_project(dir.path());
        std::fs::create_dir_all(dir.path().join("assets/character/mock")).unwrap();
        std::fs::write(
            dir.path().join("assets/character/mock/lipsync-001.json"),
            b"{}",
        )
        .unwrap();
        project.tracks.push(Track {
            id: "character_performance".into(),
            kind: TrackKind::CharacterPerformance,
            clips: vec![Clip::CharacterPerformance(CharacterPerformanceClip {
                id: "cp-001".into(),
                start_ms: 0,
                duration_ms: 1000,
                character: "mock".into(),
                expression: "default".into(),
                motion: "idle".into(),
                lip_sync: RelativeAssetPath::new("assets/character/mock/lipsync-001.json").unwrap(),
                transform: Transform::default(),
                extra: BTreeMap::new(),
            })],
        });

        let registry = build_registry(&project, dir.path(), None);
        let cp = registry
            .assets
            .iter()
            .find(|a| a.id == "generated.cp-001")
            .unwrap();
        assert_eq!(cp.kind, AssetKind::Generated);
        assert_eq!(cp.provenance, PROVENANCE_GENERATED);
        assert!(cp.exists());
    }

    #[test]
    fn character_manifest_entries_only_cover_referenced_characters() {
        let manifest_dir = tempfile::tempdir().unwrap();
        let sprites = manifest_dir.path().join("sprites");
        std::fs::create_dir_all(&sprites).unwrap();
        for name in ["closed.png", "half.png", "open.png"] {
            std::fs::write(sprites.join(name), b"fake png").unwrap();
        }
        let manifest = CharacterManifest::parse(
            "characters:\n  - id: mock_a\n    display_name: A\n    model:\n      type: png_lipsync\n      closed: sprites/closed.png\n      half: sprites/half.png\n      open: sprites/open.png\n  - id: unused\n    display_name: Unused\n",
            manifest_dir.path(),
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut project = sample_project(dir.path());
        project.tracks.push(Track {
            id: "character_performance".into(),
            kind: TrackKind::CharacterPerformance,
            clips: vec![Clip::CharacterPerformance(CharacterPerformanceClip {
                id: "cp-001".into(),
                start_ms: 0,
                duration_ms: 1000,
                character: "mock_a".into(),
                expression: "default".into(),
                motion: "idle".into(),
                lip_sync: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                transform: Transform::default(),
                extra: BTreeMap::new(),
            })],
        });

        let registry = build_registry(&project, dir.path(), Some((&manifest, manifest_dir.path())));
        let character_entries: Vec<&AssetRecord> = registry
            .assets
            .iter()
            .filter(|a| a.kind == AssetKind::Character)
            .collect();
        assert_eq!(character_entries.len(), 1, "only the referenced character");
        let a = character_entries[0];
        assert_eq!(a.id, "character.mock_a");
        assert_eq!(a.files.len(), 3);
        assert!(a.exists());
    }

    #[test]
    fn save_and_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let project = sample_project(dir.path());
        let registry = build_registry(&project, dir.path(), None);
        let path = dir.path().join("asset-registry.json");
        registry.save(&path).unwrap();
        let back = AssetRegistry::load(&path).unwrap();
        assert_eq!(back, registry);
    }
}
