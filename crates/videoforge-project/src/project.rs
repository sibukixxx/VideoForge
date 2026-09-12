use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::path::RelativeAssetPath;
use crate::presentation::Presentation;

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
    /// Background music. Long, usually looping, laid under the whole timeline.
    Bgm,
    /// Live2D/VOICEVOX character performance data (expression, motion,
    /// lip-sync curve) — distinct from `Character` (立ち絵 stand-in images).
    CharacterPerformance,
    /// A general video clip (P1-2) — distinct from `Character`/`Image`,
    /// which are always still images.
    Video,
    /// On-screen text (titles, labels) — distinct from `Caption`, which is
    /// always tied to a dialogue's speaker/timing (P1-1).
    Text,
}

/// A clip on a track. Tagged by `type` so exporters can dispatch on it.
///
/// There is no separate "OverlayClip" variant (design sketch in P1-1):
/// an overlay (a watermark, a lower-third, a callout image) is exactly an
/// [`ImageClip`] with `presentation.role` set to `"overlay"` — layering,
/// position and opacity already come from the same [`Transform`] every
/// visual clip carries, so a second type would duplicate `ImageClip`
/// field-for-field rather than add anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Clip {
    Audio(AudioClip),
    Caption(CaptionClip),
    Background(BackgroundClip),
    Image(ImageClip),
    Character(CharacterClip),
    Bgm(BgmClip),
    SoundEffect(SoundEffectClip),
    CharacterPerformance(CharacterPerformanceClip),
    Video(VideoClip),
    Text(TextClip),
}

impl Clip {
    pub fn id(&self) -> &str {
        match self {
            Clip::Audio(c) => &c.id,
            Clip::Caption(c) => &c.id,
            Clip::Background(c) => &c.id,
            Clip::Image(c) => &c.id,
            Clip::Character(c) => &c.id,
            Clip::Bgm(c) => &c.id,
            Clip::SoundEffect(c) => &c.id,
            Clip::CharacterPerformance(c) => &c.id,
            Clip::Video(c) => &c.id,
            Clip::Text(c) => &c.id,
        }
    }

    pub fn start_ms(&self) -> u64 {
        match self {
            Clip::Audio(c) => c.start_ms,
            Clip::Caption(c) => c.start_ms,
            Clip::Background(c) => c.start_ms,
            Clip::Image(c) => c.start_ms,
            Clip::Character(c) => c.start_ms,
            Clip::Bgm(c) => c.start_ms,
            Clip::SoundEffect(c) => c.start_ms,
            Clip::CharacterPerformance(c) => c.start_ms,
            Clip::Video(c) => c.start_ms,
            Clip::Text(c) => c.start_ms,
        }
    }

    pub fn duration_ms(&self) -> u64 {
        match self {
            Clip::Audio(c) => c.duration_ms,
            Clip::Caption(c) => c.duration_ms,
            Clip::Background(c) => c.duration_ms,
            Clip::Image(c) => c.duration_ms,
            Clip::Character(c) => c.duration_ms,
            Clip::Bgm(c) => c.duration_ms,
            Clip::SoundEffect(c) => c.duration_ms,
            Clip::CharacterPerformance(c) => c.duration_ms,
            Clip::Video(c) => c.duration_ms,
            Clip::Text(c) => c.duration_ms,
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
            Clip::Image(c) => Some(&c.source),
            Clip::Character(c) => Some(&c.source),
            Clip::Bgm(c) => Some(&c.source),
            Clip::SoundEffect(c) => Some(&c.source),
            Clip::CharacterPerformance(c) => Some(&c.lip_sync),
            Clip::Video(c) => Some(&c.source),
            Clip::Text(_) => None,
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

/// How a visual clip is fitted into the video frame before `Transform::scale`
/// is applied. Closed set on purpose: exporters must be able to map every
/// value, and renderer-specific modes belong in `extra`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// Scale uniformly so the whole image is visible inside the frame.
    #[default]
    Contain,
    /// Scale uniformly so the image fills the frame, cropping the overflow.
    Cover,
    /// Scale non-uniformly to exactly the frame size.
    Stretch,
    /// Keep the image's native pixel size.
    None,
}

/// A rectangular region of a source image/frame to keep, before `fit`/`scale`
/// are applied — normalized to the *source*, not the output frame: `0,0` is
/// its top-left corner, `1,1` its bottom-right. `width`/`height` are
/// fractions of the source's own size, so `{x:0,y:0,width:1,height:1}` (not
/// the same as omitting `crop`, but equivalent in effect) keeps the whole
/// source.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Static placement of a visual clip (image, character, video) in the frame.
///
/// Coordinate system, independent of any NLE:
/// * `x`, `y` are the clip's **centre**, normalized to the frame:
///   `0.0` = left/top edge, `1.0` = right/bottom edge, `0.5, 0.5` = frame centre.
/// * `scale` multiplies the size produced by `fit` (`1.0` = unchanged).
/// * `rotation_deg` is clockwise around the centre.
/// * `opacity` is `0.0` (invisible) … `1.0` (opaque).
/// * `layer` is the z-order; a larger value is drawn in front.
/// * `crop`, when set, is applied to the source *before* `fit`/`scale` (P1-1).
///
/// Every field is a plain value: no keyframes, easing or anything that varies
/// over time. Time-dependent presentation (fade, slide, …) is expressed as
/// intent on the clip and interpolated by the renderer, so this struct never
/// has to grow a per-tool animation model.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotation_deg: f32,
    pub opacity: f32,
    pub layer: i32,
    pub fit: FitMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<CropRect>,
}

impl Default for Transform {
    /// Centred, fitted, fully opaque, unrotated, on layer 0, uncropped.
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            scale: 1.0,
            rotation_deg: 0.0,
            opacity: 1.0,
            layer: 0,
            fit: FitMode::Contain,
            crop: None,
        }
    }
}

/// A still image (screenshot, diagram, slide) shown for a span of time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default)]
    pub transform: Transform,
    /// Semantic role / intent (issue #16); `None` when the author said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// A character stand-in (立ち絵): an image that belongs to a speaker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Canonical speaker key (e.g. `reimu`) this stand-in represents, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default)]
    pub transform: Transform,
    /// Semantic role / intent (issue #16); `None` when the author said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// A video clip (P1-2): a general video asset placed on the timeline,
/// distinct from `Character`/`Image` (always still images).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoClip {
    pub id: String,
    pub source: RelativeAssetPath,
    /// Where this clip sits on the *output* timeline.
    pub start_ms: u64,
    /// How long this clip plays on the output timeline. May exceed the
    /// source's own remaining length after `trim_start_ms` only when
    /// `looping` is set — otherwise the renderer holds the last frame for
    /// the remainder (same "never desync the timeline" rule as any other
    /// clip; see `videoforge-timeline`'s placement rules).
    pub duration_ms: u64,
    /// In-point within the source file — the source plays starting from
    /// this offset, not from its own beginning.
    #[serde(default)]
    pub trim_start_ms: u64,
    /// Linear gain applied to the source's own audio track, `1.0` = as
    /// authored, `0.0` = silent regardless of `muted`.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Drop the source's audio track entirely (distinct from `volume: 0.0`
    /// so a renderer need not decode/mix audio it will discard).
    #[serde(default)]
    pub muted: bool,
    /// Repeat the source from `trim_start_ms` to fill `duration_ms` when the
    /// source is shorter, instead of holding the last frame.
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub transform: Transform,
    /// Semantic role / intent (issue #16); `None` when the author said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// On-screen text (a title, a label) — distinct from `CaptionClip`, which is
/// always tied to a dialogue's speaker/timing (P1-1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextClip {
    pub id: String,
    pub text: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default)]
    pub transform: Transform,
    /// Hex color (e.g. `#ffffff`); `None` uses the renderer's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Semantic role / intent (issue #16); `None` when the author said nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// One dialogue's Live2D/VOICEVOX character performance: which character,
/// what expression/motion it plays, and a reference to its deterministic
/// lip-sync amplitude curve (design §9, §10, §11). Distinct from
/// `CharacterClip`, which is a static stand-in *image* (立ち絵) — this clip
/// has no `source`/visual asset of its own; a renderer combines it with the
/// character's Live2D model (resolved separately, outside the project IR
/// per design §6) to produce frames in Phase 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharacterPerformanceClip {
    pub id: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Character id from the linked character manifest (design §5), e.g. `tsumugi`.
    pub character: String,
    /// `"default"` when the script did not say (design §12).
    #[serde(default = "default_expression")]
    pub expression: String,
    /// `"idle"` when the script did not say (design §12).
    #[serde(default = "default_motion")]
    pub motion: String,
    /// Deterministic, offline-renderable lip-sync curve for this dialogue's
    /// audio (`core::lipsync::LipSyncTrack`, referenced rather than inlined
    /// so `project.vfp.json` stays small).
    pub lip_sync: RelativeAssetPath,
    /// On-screen placement (P0-1), derived once from the character
    /// manifest's `presentation` and repeated on every clip for that
    /// character — same field, same coordinate system, as `ImageClip`/
    /// `CharacterClip::transform`, not a second placement concept.
    /// `#[serde(default)]` so older `project.vfp.json` files without this
    /// field still load (additive, no `SCHEMA_VERSION` bump).
    #[serde(default)]
    pub transform: Transform,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_expression() -> String {
    "default".to_string()
}

fn default_motion() -> String {
    "idle".to_string()
}

/// Background music. Audio only: no placement in the frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BgmClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Linear gain, `1.0` = as authored — this is the *base* level; the
    /// renderer additionally ducks it under any overlapping dialogue
    /// (P1-4), so this is "as loud as it gets", not "as loud as it always
    /// plays".
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Repeat the source until `duration_ms` is filled instead of going silent.
    #[serde(default)]
    pub looping: bool,
    /// In-point within the source file (P1-4), same rule as `VideoClip::trim_start_ms`.
    #[serde(default)]
    pub trim_start_ms: u64,
    /// Linear ramp up from silence at the clip's own start.
    #[serde(default)]
    pub fade_in_ms: u64,
    /// Linear ramp down to silence at the clip's own end.
    #[serde(default)]
    pub fade_out_ms: u64,
    /// Apply loudness normalization (FFmpeg `dynaudnorm`) to this clip alone.
    #[serde(default)]
    pub normalize: bool,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// A one-shot sound effect. Audio only: no placement in the frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundEffectClip {
    pub id: String,
    pub source: RelativeAssetPath,
    pub start_ms: u64,
    pub duration_ms: u64,
    /// Linear gain, `1.0` = as authored.
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_volume() -> f32 {
    1.0
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

    pub fn image_clips(&self) -> Vec<&ImageClip> {
        self.tracks_of(TrackKind::Image)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Image(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn character_clips(&self) -> Vec<&CharacterClip> {
        self.tracks_of(TrackKind::Character)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Character(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn character_performance_clips(&self) -> Vec<&CharacterPerformanceClip> {
        self.tracks_of(TrackKind::CharacterPerformance)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::CharacterPerformance(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn bgm_clips(&self) -> Vec<&BgmClip> {
        self.tracks_of(TrackKind::Bgm)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Bgm(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn sound_effect_clips(&self) -> Vec<&SoundEffectClip> {
        self.tracks_of(TrackKind::SoundEffect)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::SoundEffect(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn video_clips(&self) -> Vec<&VideoClip> {
        self.tracks_of(TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Video(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    pub fn text_clips(&self) -> Vec<&TextClip> {
        self.tracks_of(TrackKind::Text)
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| match c {
                Clip::Text(a) => Some(a),
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

    fn visual_and_media_project() -> VideoProject {
        let mut p = sample();
        p.tracks.push(Track {
            id: "image".into(),
            kind: TrackKind::Image,
            clips: vec![Clip::Image(ImageClip {
                id: "image-001".into(),
                source: RelativeAssetPath::new("assets/image/rust.png").unwrap(),
                start_ms: 500,
                duration_ms: 2000,
                transform: Transform {
                    x: 0.75,
                    y: 0.25,
                    scale: 0.5,
                    rotation_deg: -3.0,
                    opacity: 0.9,
                    layer: 10,
                    fit: FitMode::Cover,
                    crop: None,
                },
                presentation: Some(Presentation {
                    role: Some("primary_visual".into()),
                    intent: Some("fade".into()),
                    intent_duration_ms: Some(300),
                }),
                extra: BTreeMap::from([("note".to_string(), serde_json::json!("hero"))]),
            })],
        });
        p.tracks.push(Track {
            id: "character".into(),
            kind: TrackKind::Character,
            clips: vec![Clip::Character(CharacterClip {
                id: "character-001".into(),
                source: RelativeAssetPath::new("assets/character/reimu/normal.png").unwrap(),
                start_ms: 0,
                duration_ms: 3410,
                speaker: Some("reimu".into()),
                transform: Transform::default(),
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "bgm".into(),
            kind: TrackKind::Bgm,
            clips: vec![Clip::Bgm(BgmClip {
                id: "bgm-001".into(),
                source: RelativeAssetPath::new("assets/bgm/main.mp3").unwrap(),
                start_ms: 0,
                duration_ms: 3410,
                volume: 0.6,
                looping: true,
                trim_start_ms: 0,
                fade_in_ms: 0,
                fade_out_ms: 0,
                normalize: false,
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "se".into(),
            kind: TrackKind::SoundEffect,
            clips: vec![Clip::SoundEffect(SoundEffectClip {
                id: "se-001".into(),
                source: RelativeAssetPath::new("assets/se/pop.wav").unwrap(),
                start_ms: 1200,
                duration_ms: 800,
                volume: 1.0,
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "video".into(),
            kind: TrackKind::Video,
            clips: vec![Clip::Video(VideoClip {
                id: "video-001".into(),
                source: RelativeAssetPath::new("assets/video/clip.mp4").unwrap(),
                start_ms: 0,
                duration_ms: 3410,
                trim_start_ms: 250,
                volume: 0.8,
                muted: false,
                looping: true,
                transform: Transform {
                    crop: Some(CropRect {
                        x: 0.1,
                        y: 0.0,
                        width: 0.8,
                        height: 1.0,
                    }),
                    ..Transform::default()
                },
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        p.tracks.push(Track {
            id: "text".into(),
            kind: TrackKind::Text,
            clips: vec![Clip::Text(TextClip {
                id: "text-001".into(),
                text: "Title Card".into(),
                start_ms: 0,
                duration_ms: 1000,
                transform: Transform::default(),
                color: Some("#ffcc00".into()),
                presentation: None,
                extra: BTreeMap::new(),
            })],
        });
        p
    }

    fn character_performance_project() -> VideoProject {
        let mut p = sample();
        p.tracks.push(Track {
            id: "character_performance".into(),
            kind: TrackKind::CharacterPerformance,
            clips: vec![Clip::CharacterPerformance(CharacterPerformanceClip {
                id: "perf-001".into(),
                start_ms: 0,
                duration_ms: 3410,
                character: "tsumugi".into(),
                expression: "smile".into(),
                motion: "wave".into(),
                lip_sync: RelativeAssetPath::new("assets/character/tsumugi/lipsync-001.json")
                    .unwrap(),
                transform: Transform::default(),
                extra: BTreeMap::new(),
            })],
        });
        p
    }

    #[test]
    fn character_performance_clip_accessors_and_defaults() {
        let p = character_performance_project();
        let clips = p.character_performance_clips();
        assert_eq!(clips.len(), 1);
        let c = clips[0];
        assert_eq!(c.character, "tsumugi");
        assert_eq!(c.expression, "smile");
        assert_eq!(c.motion, "wave");

        let clip = Clip::CharacterPerformance(c.clone());
        assert_eq!(clip.id(), "perf-001");
        assert_eq!(clip.start_ms(), 0);
        assert_eq!(clip.duration_ms(), 3410);
        assert_eq!(clip.end_ms(), 3410);
        assert_eq!(
            clip.asset().unwrap().as_str(),
            "assets/character/tsumugi/lipsync-001.json"
        );

        let assets = p.referenced_assets();
        assert!(assets
            .iter()
            .any(|a| a.as_str() == "assets/character/tsumugi/lipsync-001.json"));
    }

    #[test]
    fn character_performance_expression_and_motion_default_when_omitted_in_json() {
        let json = r#"{"schema_version": 1, "id": "x", "title": "X",
            "video": {"width": 1, "height": 1, "fps": 1}, "source": {},
            "tracks": [
              {"id": "cp", "kind": "character_performance", "clips": [
                {"type": "character_performance", "id": "p1", "character": "tsumugi",
                 "start_ms": 0, "duration_ms": 1000,
                 "lip_sync": "assets/character/tsumugi/lipsync-001.json"}]}
            ]}"#;
        let p = VideoProject::from_json(json).unwrap();
        let c = &p.character_performance_clips()[0];
        assert_eq!(c.expression, "default");
        assert_eq!(c.motion, "idle");
    }

    #[test]
    fn character_performance_roundtrips_and_preserves_unknown_fields() {
        let mut p = character_performance_project();
        let track = p
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::CharacterPerformance)
            .unwrap();
        if let Clip::CharacterPerformance(c) = &mut track.clips[0] {
            c.extra
                .insert("phoneme_hint".into(), serde_json::json!("a"));
        }
        let json = p.to_json().unwrap();
        assert!(json.contains("\"type\": \"character_performance\""));
        assert!(json.contains("\"kind\": \"character_performance\""));

        let back = VideoProject::from_json(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(
            back.schema_version, 1,
            "additive change must not bump the schema"
        );
    }

    #[test]
    fn clip_helpers_cover_visual_and_media_variants() {
        let p = visual_and_media_project();

        let image = p.image_clips()[0];
        let character = p.character_clips()[0];
        let bgm = p.bgm_clips()[0];
        let se = p.sound_effect_clips()[0];
        let video = p.video_clips()[0];
        let text = p.text_clips()[0];
        assert_eq!(image.id, "image-001");
        assert_eq!(character.speaker.as_deref(), Some("reimu"));
        assert_eq!(bgm.volume, 0.6);
        assert_eq!(se.duration_ms, 800);
        assert_eq!(video.trim_start_ms, 250);
        assert_eq!(video.transform.crop.unwrap().width, 0.8);
        assert_eq!(text.text, "Title Card");
        assert_eq!(text.color.as_deref(), Some("#ffcc00"));

        let by_id: BTreeMap<&str, &Clip> = p.clips().map(|c| (c.id(), c)).collect();
        let image = by_id["image-001"];
        assert_eq!(image.start_ms(), 500);
        assert_eq!(image.duration_ms(), 2000);
        assert_eq!(image.end_ms(), 2500);
        assert_eq!(image.asset().unwrap().as_str(), "assets/image/rust.png");
        assert_eq!(
            by_id["character-001"].asset().unwrap().as_str(),
            "assets/character/reimu/normal.png"
        );
        assert_eq!(
            by_id["bgm-001"].asset().unwrap().as_str(),
            "assets/bgm/main.mp3"
        );
        assert_eq!(by_id["se-001"].end_ms(), 2000);
    }

    #[test]
    fn roundtrip_json_with_mixed_clips_preserves_every_field() {
        let mut p = visual_and_media_project();
        p.extra
            .insert("future_field".into(), serde_json::json!({"a": 1}));

        let json = p.to_json().unwrap();
        assert!(json.contains("\"type\": \"image\""));
        assert!(json.contains("\"type\": \"character\""));
        assert!(json.contains("\"type\": \"bgm\""));
        assert!(json.contains("\"type\": \"sound_effect\""));
        assert!(json.contains("\"type\": \"video\""));
        assert!(json.contains("\"type\": \"text\""));
        assert!(json.contains("\"kind\": \"bgm\""));
        assert!(json.contains("\"kind\": \"video\""));
        assert!(json.contains("\"kind\": \"text\""));
        assert!(json.contains("\"fit\": \"cover\""));
        assert!(json.contains("\"role\": \"primary_visual\""));
        let character_json = serde_json::to_string(&p.character_clips()[0]).unwrap();
        assert!(
            !character_json.contains("presentation"),
            "None must be omitted: {character_json}"
        );

        let back = VideoProject::from_json(&json).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.to_json().unwrap(), json);
        assert_eq!(
            back.schema_version, 1,
            "additive change must not bump the schema"
        );
    }

    #[test]
    fn referenced_assets_include_visual_and_media_clips() {
        let p = visual_and_media_project();
        let assets = p.referenced_assets();
        let assets: Vec<&str> = assets.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            assets,
            vec![
                "assets/audio/001.wav",
                "assets/bgm/main.mp3",
                "assets/character/reimu/normal.png",
                "assets/image/rust.png",
                "assets/se/pop.wav",
                "assets/video/clip.mp4",
            ]
        );
    }

    #[test]
    fn transform_and_volume_default_when_omitted_in_json() {
        let json = r#"{"schema_version": 1, "id": "x", "title": "X",
            "video": {"width": 1, "height": 1, "fps": 1}, "source": {},
            "tracks": [
              {"id": "i", "kind": "image", "clips": [
                {"type": "image", "id": "i1", "source": "assets/image/a.png",
                 "start_ms": 0, "duration_ms": 1, "transform": {"layer": 3}}]},
              {"id": "b", "kind": "bgm", "clips": [
                {"type": "bgm", "id": "b1", "source": "assets/bgm/a.mp3",
                 "start_ms": 0, "duration_ms": 1}]}
            ]}"#;
        let p = VideoProject::from_json(json).unwrap();
        let image = p.image_clips()[0];
        assert_eq!(image.transform.layer, 3);
        assert_eq!(image.transform.x, 0.5);
        assert_eq!(image.transform.y, 0.5);
        assert_eq!(image.transform.scale, 1.0);
        assert_eq!(image.transform.rotation_deg, 0.0);
        assert_eq!(image.transform.opacity, 1.0);
        assert_eq!(image.transform.fit, FitMode::Contain);
        assert_eq!(image.presentation, None);
        let bgm = p.bgm_clips()[0];
        assert_eq!(bgm.volume, 1.0);
        assert!(!bgm.looping);
    }

    #[test]
    fn rejects_unknown_fit_mode() {
        let json = r#"{"schema_version": 1, "id": "x", "title": "X",
            "video": {"width": 1, "height": 1, "fps": 1}, "source": {},
            "tracks": [{"id": "i", "kind": "image", "clips": [
                {"type": "image", "id": "i1", "source": "assets/image/a.png",
                 "start_ms": 0, "duration_ms": 1, "transform": {"fit": "anchor_point"}}]}]}"#;
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
