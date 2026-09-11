//! Canonical VideoProject IR.
//!
//! `project.vfp.json` is the single source of truth in VideoForge. Every
//! exporter (`.ymmp`, FFmpeg preview, future FCPXML / OTIO) derives from it.
//!
//! Design rules enforced here:
//! * Time is stored in milliseconds. Frames are an exporter concern
//!   (see [`time::millis_to_frame`]).
//! * Asset references are [`RelativeAssetPath`]s, relative to the directory
//!   that contains the project file. Absolute paths (Windows or POSIX) are
//!   rejected at construction and at deserialization time.
//! * Unknown fields are preserved so that future schema additions round-trip.

pub mod path;
pub mod presentation;
pub mod project;
pub mod time;
pub mod validation;

pub use path::{PathError, RelativeAssetPath};
pub use presentation::{Presentation, KNOWN_INTENTS, KNOWN_ROLES};
pub use project::{
    AudioClip, BackgroundClip, BgmClip, CaptionClip, CharacterClip, Clip, FitMode, ImageClip,
    ProjectError, SoundEffectClip, SourceInfo, Track, TrackKind, Transform, VideoProject,
    VideoSettings, SCHEMA_VERSION,
};
pub use time::{format_srt_timestamp, format_timespan, frame_to_millis, millis_to_frame};
pub use validation::{
    validate_project, ProjectValidationIssue, ProjectValidationReport, ValidationSeverity,
};
