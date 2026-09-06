//! Timeline builder: schedules dialogues sequentially using real WAV
//! durations, inserts a configurable gap, and emits a [`VideoProject`].
//!
//! ```text
//! Dialogue 1: 0     - 3410ms
//! Gap:        3410  - 3610ms
//! Dialogue 2: 3610  - 7290ms
//! ```

pub mod srt;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use videoforge_project::{
    AudioClip, BackgroundClip, CaptionClip, Clip, RelativeAssetPath, SourceInfo, Track, TrackKind,
    VideoProject, VideoSettings,
};

#[derive(Debug, Error)]
pub enum TimelineError {
    #[error("timeline has no dialogues")]
    Empty,
    #[error("dialogue {index} has zero duration")]
    ZeroDuration { index: usize },
}

/// One synthesized dialogue ready to be scheduled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DialogueInput {
    /// 1-based dialogue number.
    pub index: usize,
    /// Canonical speaker key (config key, e.g. `reimu`).
    pub speaker: String,
    /// Display name as written in the script (e.g. `霊夢`).
    pub speaker_display: String,
    pub text: String,
    pub audio: RelativeAssetPath,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineOptions {
    /// Silence inserted between consecutive dialogues.
    pub dialogue_gap_ms: u64,
}

impl Default for TimelineOptions {
    fn default() -> Self {
        Self {
            dialogue_gap_ms: 200,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineInput {
    pub id: String,
    pub title: String,
    pub video: VideoSettings,
    pub source: SourceInfo,
    pub dialogues: Vec<DialogueInput>,
    /// Optional background image spanning the whole timeline.
    pub background: Option<RelativeAssetPath>,
    pub options: TimelineOptions,
}

/// Where each dialogue landed on the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledDialogue {
    pub index: usize,
    pub speaker: String,
    pub speaker_display: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Pure scheduling step: returns `(start, end)` per dialogue in order.
pub fn schedule(
    dialogues: &[DialogueInput],
    options: &TimelineOptions,
) -> Result<Vec<ScheduledDialogue>, TimelineError> {
    if dialogues.is_empty() {
        return Err(TimelineError::Empty);
    }
    let mut cursor = 0u64;
    let mut out = Vec::with_capacity(dialogues.len());
    for (i, d) in dialogues.iter().enumerate() {
        if d.duration_ms == 0 {
            return Err(TimelineError::ZeroDuration { index: d.index });
        }
        if i > 0 {
            cursor += options.dialogue_gap_ms;
        }
        let start = cursor;
        let end = start + d.duration_ms;
        out.push(ScheduledDialogue {
            index: d.index,
            speaker: d.speaker.clone(),
            speaker_display: d.speaker_display.clone(),
            start_ms: start,
            end_ms: end,
        });
        cursor = end;
    }
    Ok(out)
}

/// Build the canonical project from synthesized dialogues.
pub fn build(input: TimelineInput) -> Result<VideoProject, TimelineError> {
    let scheduled = schedule(&input.dialogues, &input.options)?;

    let mut audio_clips = Vec::with_capacity(scheduled.len());
    let mut caption_clips = Vec::with_capacity(scheduled.len());
    for (d, s) in input.dialogues.iter().zip(scheduled.iter()) {
        let suffix = format!("{:03}", d.index);
        audio_clips.push(Clip::Audio(AudioClip {
            id: format!("audio-{suffix}"),
            source: d.audio.clone(),
            start_ms: s.start_ms,
            duration_ms: d.duration_ms,
            speaker: d.speaker.clone(),
            extra: BTreeMap::new(),
        }));
        caption_clips.push(Clip::Caption(CaptionClip {
            id: format!("caption-{suffix}"),
            text: d.text.clone(),
            start_ms: s.start_ms,
            duration_ms: d.duration_ms,
            speaker: d.speaker.clone(),
            speaker_display: Some(d.speaker_display.clone()),
            extra: BTreeMap::new(),
        }));
    }
    let total = scheduled.last().map(|s| s.end_ms).unwrap_or(0);

    let mut project = VideoProject::new(input.id, input.title, input.video);
    project.source = input.source;

    if let Some(bg) = input.background {
        project.tracks.push(Track {
            id: "background".into(),
            kind: TrackKind::Background,
            clips: vec![Clip::Background(BackgroundClip {
                id: "background-001".into(),
                source: bg,
                start_ms: 0,
                duration_ms: total,
                extra: BTreeMap::new(),
            })],
        });
    }
    project.tracks.push(Track {
        id: "audio".into(),
        kind: TrackKind::Audio,
        clips: audio_clips,
    });
    project.tracks.push(Track {
        id: "caption".into(),
        kind: TrackKind::Caption,
        clips: caption_clips,
    });
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogue(index: usize, speaker: &str, duration_ms: u64) -> DialogueInput {
        DialogueInput {
            index,
            speaker: speaker.into(),
            speaker_display: speaker.to_uppercase(),
            text: format!("text {index}"),
            audio: RelativeAssetPath::new(format!("assets/audio/{index:03}.wav")).unwrap(),
            duration_ms,
        }
    }

    #[test]
    fn schedules_with_gap() {
        let s = schedule(
            &[dialogue(1, "reimu", 3410), dialogue(2, "marisa", 3680)],
            &TimelineOptions::default(),
        )
        .unwrap();
        assert_eq!((s[0].start_ms, s[0].end_ms), (0, 3410));
        assert_eq!((s[1].start_ms, s[1].end_ms), (3610, 7290));
    }

    #[test]
    fn zero_gap_is_contiguous() {
        let s = schedule(
            &[
                dialogue(1, "a", 100),
                dialogue(2, "b", 100),
                dialogue(3, "c", 100),
            ],
            &TimelineOptions { dialogue_gap_ms: 0 },
        )
        .unwrap();
        assert_eq!(s[2].start_ms, 200);
        assert_eq!(s[2].end_ms, 300);
    }

    #[test]
    fn rejects_empty_and_zero_duration() {
        assert!(matches!(
            schedule(&[], &TimelineOptions::default()),
            Err(TimelineError::Empty)
        ));
        assert!(matches!(
            schedule(&[dialogue(1, "a", 0)], &TimelineOptions::default()),
            Err(TimelineError::ZeroDuration { index: 1 })
        ));
    }

    #[test]
    fn builds_project_with_tracks() {
        let project = build(TimelineInput {
            id: "sample".into(),
            title: "Sample".into(),
            video: VideoSettings::default(),
            source: SourceInfo::default(),
            dialogues: vec![dialogue(1, "reimu", 3410), dialogue(2, "marisa", 3680)],
            background: Some(RelativeAssetPath::new("assets/background/default.png").unwrap()),
            options: TimelineOptions::default(),
        })
        .unwrap();

        assert_eq!(project.tracks.len(), 3);
        assert_eq!(project.total_duration_ms(), 7290);
        let audio = project.audio_clips();
        assert_eq!(audio[1].start_ms, 3610);
        assert_eq!(audio[1].id, "audio-002");
        let captions = project.caption_clips();
        assert_eq!(captions[1].speaker_display.as_deref(), Some("MARISA"));
        let bg = project.background_clips();
        assert_eq!(bg[0].duration_ms, 7290);
        assert_eq!(project.referenced_assets().len(), 3);
    }
}
