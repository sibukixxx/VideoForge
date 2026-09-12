//! Pure structural and semantic validation for the canonical VideoProject IR.

use std::collections::{hash_map::Entry, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Clip, TrackKind, VideoProject};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectValidationIssue {
    pub severity: ValidationSeverity,
    pub code: String,
    /// JSONPath-like location in `project.vfp.json`.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectValidationReport {
    pub errors: Vec<ProjectValidationIssue>,
    pub warnings: Vec<ProjectValidationIssue>,
}

impl ProjectValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    fn push(&mut self, severity: ValidationSeverity, code: &str, path: String, message: String) {
        let issue = ProjectValidationIssue {
            severity,
            code: code.into(),
            path,
            message,
        };
        match severity {
            ValidationSeverity::Error => self.errors.push(issue),
            ValidationSeverity::Warning => self.warnings.push(issue),
        }
    }

    fn error(&mut self, code: &str, path: String, message: String) {
        self.push(ValidationSeverity::Error, code, path, message);
    }

    fn warning(&mut self, code: &str, path: String, message: String) {
        self.push(ValidationSeverity::Warning, code, path, message);
    }
}

/// Validate an already parsed VideoProject without filesystem or runtime access.
pub fn validate_project(project: &VideoProject) -> ProjectValidationReport {
    let mut report = ProjectValidationReport::default();
    validate_video_settings(project, &mut report);

    let mut track_ids = HashSet::new();
    let mut clip_ids: HashMap<&str, String> = HashMap::new();
    let mut dialogue_end_ms = None;

    for (track_index, track) in project.tracks.iter().enumerate() {
        let track_path = format!("$.tracks[{track_index}]");
        if track.id.trim().is_empty() {
            report.error(
                "empty_track_id",
                format!("{track_path}.id"),
                "track id must not be empty".into(),
            );
        } else if !track_ids.insert(track.id.as_str()) {
            report.error(
                "duplicate_track_id",
                format!("{track_path}.id"),
                format!("track id `{}` is duplicated", track.id),
            );
        }

        for (clip_index, clip) in track.clips.iter().enumerate() {
            let clip_path = format!("{track_path}.clips[{clip_index}]");
            let clip_id = clip.id();
            if clip_id.trim().is_empty() {
                report.error(
                    "empty_clip_id",
                    format!("{clip_path}.id"),
                    "clip id must not be empty".into(),
                );
            } else {
                match clip_ids.entry(clip_id) {
                    Entry::Vacant(entry) => {
                        entry.insert(clip_path.clone());
                    }
                    Entry::Occupied(entry) => report.error(
                        "duplicate_clip_id",
                        format!("{clip_path}.id"),
                        format!("clip id `{clip_id}` is already used at {}", entry.get()),
                    ),
                }
            }

            let actual_kind = clip_kind(clip);
            if track.kind != actual_kind {
                report.error(
                    "track_clip_kind_mismatch",
                    format!("{clip_path}.type"),
                    format!(
                        "clip type `{}` does not match track kind `{}`",
                        kind_name(actual_kind),
                        kind_name(track.kind)
                    ),
                );
            }
            if clip.duration_ms() == 0 {
                report.error(
                    "zero_clip_duration",
                    format!("{clip_path}.duration_ms"),
                    "clip duration must be greater than zero".into(),
                );
            }
            match clip.start_ms().checked_add(clip.duration_ms()) {
                Some(end_ms) if matches!(actual_kind, TrackKind::Audio | TrackKind::Caption) => {
                    dialogue_end_ms =
                        Some(dialogue_end_ms.map_or(end_ms, |end: u64| end.max(end_ms)));
                }
                Some(_) => {}
                None => report.error(
                    "clip_time_overflow",
                    clip_path,
                    "start_ms + duration_ms exceeds u64".into(),
                ),
            }
        }
    }

    validate_dialogue_pairs(project, &mut report);
    if let Some(end_ms) = dialogue_end_ms {
        validate_visual_bounds(project, end_ms, &mut report);
    }
    report
}

fn validate_video_settings(project: &VideoProject, report: &mut ProjectValidationReport) {
    for (field, value) in [
        ("width", project.video.width),
        ("height", project.video.height),
        ("fps", project.video.fps),
    ] {
        if value == 0 {
            report.error(
                "zero_video_setting",
                format!("$.video.{field}"),
                format!("video {field} must be greater than zero"),
            );
        }
    }
}

fn validate_dialogue_pairs(project: &VideoProject, report: &mut ProjectValidationReport) {
    let audio = project.audio_clips();
    for (track_index, track) in project.tracks.iter().enumerate() {
        for (clip_index, clip) in track.clips.iter().enumerate() {
            if let Clip::Caption(caption) = clip {
                let paired = audio.iter().any(|audio| {
                    audio.speaker == caption.speaker
                        && audio.start_ms == caption.start_ms
                        && audio.duration_ms == caption.duration_ms
                });
                if !paired {
                    report.warning(
                        "caption_without_matching_audio",
                        format!("$.tracks[{track_index}].clips[{clip_index}]"),
                        format!(
                            "caption `{}` has no audio with the same speaker and timing",
                            caption.id
                        ),
                    );
                }
            }
        }
    }
}

fn validate_visual_bounds(
    project: &VideoProject,
    dialogue_end_ms: u64,
    report: &mut ProjectValidationReport,
) {
    for (track_index, track) in project.tracks.iter().enumerate() {
        for (clip_index, clip) in track.clips.iter().enumerate() {
            let kind = clip_kind(clip);
            if matches!(kind, TrackKind::Audio | TrackKind::Caption) {
                continue;
            }
            if clip
                .start_ms()
                .checked_add(clip.duration_ms())
                .is_some_and(|end_ms| end_ms > dialogue_end_ms)
            {
                report.warning(
                    "visual_after_dialogue_timeline",
                    format!("$.tracks[{track_index}].clips[{clip_index}]"),
                    format!(
                        "clip `{}` ends after the dialogue timeline at {dialogue_end_ms} ms",
                        clip.id()
                    ),
                );
            }
        }
    }
}

fn clip_kind(clip: &Clip) -> TrackKind {
    match clip {
        Clip::Audio(_) => TrackKind::Audio,
        Clip::Caption(_) => TrackKind::Caption,
        Clip::Background(_) => TrackKind::Background,
        Clip::Image(_) => TrackKind::Image,
        Clip::Character(_) => TrackKind::Character,
        Clip::Bgm(_) => TrackKind::Bgm,
        Clip::SoundEffect(_) => TrackKind::SoundEffect,
        Clip::CharacterPerformance(_) => TrackKind::CharacterPerformance,
        Clip::Video(_) => TrackKind::Video,
        Clip::Text(_) => TrackKind::Text,
    }
}

fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio",
        TrackKind::Caption => "caption",
        TrackKind::Character => "character",
        TrackKind::Image => "image",
        TrackKind::Background => "background",
        TrackKind::SoundEffect => "sound_effect",
        TrackKind::Bgm => "bgm",
        TrackKind::CharacterPerformance => "character_performance",
        TrackKind::Video => "video",
        TrackKind::Text => "text",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        AudioClip, CaptionClip, CharacterPerformanceClip, RelativeAssetPath, Track, VideoSettings,
    };

    fn valid_project() -> VideoProject {
        let mut project = VideoProject::new("sample", "Sample", VideoSettings::default());
        project.tracks = vec![
            Track {
                id: "audio".into(),
                kind: TrackKind::Audio,
                clips: vec![Clip::Audio(AudioClip {
                    id: "audio-001".into(),
                    source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                    start_ms: 0,
                    duration_ms: 1000,
                    speaker: "reimu".into(),
                    extra: BTreeMap::new(),
                })],
            },
            Track {
                id: "caption".into(),
                kind: TrackKind::Caption,
                clips: vec![Clip::Caption(CaptionClip {
                    id: "caption-001".into(),
                    text: "こんにちは".into(),
                    start_ms: 0,
                    duration_ms: 1000,
                    speaker: "reimu".into(),
                    speaker_display: Some("霊夢".into()),
                    extra: BTreeMap::new(),
                })],
            },
        ];
        project
    }

    #[test]
    fn accepts_a_valid_generated_shape() {
        assert!(validate_project(&valid_project()).is_ok());
    }

    #[test]
    fn accepts_character_performance_on_its_matching_track() {
        let mut project = valid_project();
        project.tracks.push(Track {
            id: "character-performance".into(),
            kind: TrackKind::CharacterPerformance,
            clips: vec![Clip::CharacterPerformance(CharacterPerformanceClip {
                id: "character-performance-001".into(),
                start_ms: 0,
                duration_ms: 1000,
                character: "tsumugi".into(),
                expression: "default".into(),
                motion: "idle".into(),
                lip_sync: RelativeAssetPath::new("assets/lipsync/001.json").unwrap(),
                transform: crate::project::Transform::default(),
                extra: BTreeMap::new(),
            })],
        });

        assert!(validate_project(&project).is_ok());
    }

    #[test]
    fn reports_stable_codes_and_paths() {
        let mut project = valid_project();
        project.video.fps = 0;
        project.tracks[1].id = "audio".into();
        project.tracks[1].kind = TrackKind::Audio;
        project.tracks[1].clips[0] = Clip::Caption(CaptionClip {
            id: "audio-001".into(),
            text: "ずれ".into(),
            start_ms: u64::MAX,
            duration_ms: 1,
            speaker: "marisa".into(),
            speaker_display: None,
            extra: BTreeMap::new(),
        });

        let report = validate_project(&project);
        let codes: Vec<&str> = report
            .errors
            .iter()
            .map(|issue| issue.code.as_str())
            .collect();
        assert_eq!(
            codes,
            vec![
                "zero_video_setting",
                "duplicate_track_id",
                "duplicate_clip_id",
                "track_clip_kind_mismatch",
                "clip_time_overflow",
            ]
        );
        assert_eq!(report.errors[0].path, "$.video.fps");
        assert_eq!(report.warnings[0].code, "caption_without_matching_audio");
    }
}
