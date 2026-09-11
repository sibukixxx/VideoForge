//! Timeline builder: schedules dialogues sequentially using real WAV
//! durations, inserts a configurable gap, and emits a [`VideoProject`].
//!
//! ```text
//! Dialogue 1: 0     - 3410ms
//! Gap:        3410  - 3610ms
//! Dialogue 2: 3610  - 7290ms
//! ```
//!
//! Visual events (design §17.1, issue #18) are placed *after* scheduling and
//! only consume its result: each starts where its anchor dialogue starts and,
//! unless the script gave a `duration_ms`, lasts
//!
//! * image: until the next image starts, else to the end of the timeline;
//! * character: until the next stand-in for the same speaker starts, else
//!   to the end;
//! * bgm: until the next bgm starts, else to the end;
//! * sound effect: [`DEFAULT_SOUND_EFFECT_MS`].
//!
//! Nothing extends past the end of the last dialogue.

pub mod srt;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use videoforge_project::{
    AudioClip, BackgroundClip, BgmClip, CaptionClip, CharacterClip, Clip, ImageClip, Presentation,
    RelativeAssetPath, SoundEffectClip, SourceInfo, Track, TrackKind, Transform, VideoProject,
    VideoSettings,
};

/// Default length of a sound effect clip when the script gives none.
pub const DEFAULT_SOUND_EFFECT_MS: u64 = 1000;

#[derive(Debug, Error)]
pub enum TimelineError {
    #[error("timeline has no dialogues")]
    Empty,
    #[error("dialogue {index} has zero duration")]
    ZeroDuration { index: usize },
    #[error("visual event is anchored to dialogue {index}, which does not exist")]
    UnknownAnchor { index: usize },
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

/// A resolved script directive waiting to be placed on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualEvent {
    /// 1-based index of the dialogue this event starts with.
    pub anchor_dialogue_index: usize,
    /// Explicit length from the script; `None` applies the default rule.
    pub duration_ms: Option<u64>,
    pub kind: VisualEventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VisualEventKind {
    Image {
        source: RelativeAssetPath,
        transform: Transform,
        presentation: Option<Presentation>,
    },
    Character {
        /// Canonical speaker key the stand-in belongs to, if it maps to one.
        speaker: Option<String>,
        source: RelativeAssetPath,
        transform: Transform,
        presentation: Option<Presentation>,
    },
    Bgm {
        source: RelativeAssetPath,
        volume: f32,
        looping: bool,
    },
    SoundEffect {
        source: RelativeAssetPath,
        volume: f32,
    },
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
    /// Image / character / bgm / sound-effect events, in script order.
    pub visual_events: Vec<VisualEvent>,
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
    for track in place_visual_events(&input.visual_events, &scheduled, total)? {
        project.tracks.push(track);
    }
    Ok(project)
}

/// Place resolved visual events using an already scheduled dialogue timeline.
///
/// Returns image / character / bgm / se tracks (only the non-empty ones),
/// applying the duration rules from the module docs.
///
/// This is public so non-native adapters can reuse the exact placement rules;
/// it performs no filesystem, network, process, or renderer access.
pub fn place_visual_events(
    events: &[VisualEvent],
    scheduled: &[ScheduledDialogue],
    total_ms: u64,
) -> Result<Vec<Track>, TimelineError> {
    let starts: Vec<u64> = events
        .iter()
        .map(|e| {
            scheduled
                .iter()
                .find(|s| s.index == e.anchor_dialogue_index)
                .map(|s| s.start_ms)
                .ok_or(TimelineError::UnknownAnchor {
                    index: e.anchor_dialogue_index,
                })
        })
        .collect::<Result<_, _>>()?;

    // The next event of the same family (and, for characters, the same
    // speaker) that starts strictly later ends this one.
    let next_start = |i: usize, same: &dyn Fn(&VisualEventKind) -> bool| -> Option<u64> {
        events
            .iter()
            .zip(&starts)
            .skip(i + 1)
            .filter(|(e, s)| **s > starts[i] && same(&e.kind))
            .map(|(_, s)| *s)
            .min()
    };

    let mut image = Vec::new();
    let mut character = Vec::new();
    let mut bgm = Vec::new();
    let mut se = Vec::new();
    for (i, (event, &start)) in events.iter().zip(&starts).enumerate() {
        let default_end = match &event.kind {
            VisualEventKind::Image { .. } => {
                next_start(i, &|k| matches!(k, VisualEventKind::Image { .. })).unwrap_or(total_ms)
            }
            VisualEventKind::Character { speaker, .. } => next_start(i, &|k| {
                matches!(k, VisualEventKind::Character { speaker: other, .. } if other == speaker)
            })
            .unwrap_or(total_ms),
            VisualEventKind::Bgm { .. } => {
                next_start(i, &|k| matches!(k, VisualEventKind::Bgm { .. })).unwrap_or(total_ms)
            }
            VisualEventKind::SoundEffect { .. } => start + DEFAULT_SOUND_EFFECT_MS,
        };
        let end = event
            .duration_ms
            .map_or(default_end, |d| start + d)
            .min(total_ms)
            .max(start);
        let duration_ms = end - start;
        let extra = BTreeMap::new();
        match event.kind.clone() {
            VisualEventKind::Image {
                source,
                transform,
                presentation,
            } => image.push(Clip::Image(ImageClip {
                id: format!("image-{:03}", image.len() + 1),
                source,
                start_ms: start,
                duration_ms,
                transform,
                presentation,
                extra,
            })),
            VisualEventKind::Character {
                speaker,
                source,
                transform,
                presentation,
            } => character.push(Clip::Character(CharacterClip {
                id: format!("character-{:03}", character.len() + 1),
                source,
                start_ms: start,
                duration_ms,
                speaker,
                transform,
                presentation,
                extra,
            })),
            VisualEventKind::Bgm {
                source,
                volume,
                looping,
            } => bgm.push(Clip::Bgm(BgmClip {
                id: format!("bgm-{:03}", bgm.len() + 1),
                source,
                start_ms: start,
                duration_ms,
                volume,
                looping,
                extra,
            })),
            VisualEventKind::SoundEffect { source, volume } => {
                se.push(Clip::SoundEffect(SoundEffectClip {
                    id: format!("se-{:03}", se.len() + 1),
                    source,
                    start_ms: start,
                    duration_ms,
                    volume,
                    extra,
                }))
            }
        }
    }

    Ok([
        ("image", TrackKind::Image, image),
        ("character", TrackKind::Character, character),
        ("bgm", TrackKind::Bgm, bgm),
        ("se", TrackKind::SoundEffect, se),
    ]
    .into_iter()
    .filter(|(_, _, clips)| !clips.is_empty())
    .map(|(id, kind, clips)| Track {
        id: id.into(),
        kind,
        clips,
    })
    .collect())
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
            visual_events: Vec::new(),
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

    fn asset(path: &str) -> RelativeAssetPath {
        RelativeAssetPath::new(path).unwrap()
    }

    fn image(anchor: usize, path: &str, duration_ms: Option<u64>) -> VisualEvent {
        VisualEvent {
            anchor_dialogue_index: anchor,
            duration_ms,
            kind: VisualEventKind::Image {
                source: asset(path),
                transform: Transform::default(),
                presentation: None,
            },
        }
    }

    fn character(anchor: usize, speaker: &str, path: &str) -> VisualEvent {
        VisualEvent {
            anchor_dialogue_index: anchor,
            duration_ms: None,
            kind: VisualEventKind::Character {
                speaker: Some(speaker.into()),
                source: asset(path),
                transform: Transform::default(),
                presentation: None,
            },
        }
    }

    /// Three dialogues: 0–1000, 1200–2200, 2400–3400 (gap 200).
    fn three_dialogues(visual_events: Vec<VisualEvent>) -> VideoProject {
        build(TimelineInput {
            id: "t".into(),
            title: "T".into(),
            video: VideoSettings::default(),
            source: SourceInfo::default(),
            dialogues: vec![
                dialogue(1, "reimu", 1000),
                dialogue(2, "marisa", 1000),
                dialogue(3, "reimu", 1000),
            ],
            background: None,
            visual_events,
            options: TimelineOptions::default(),
        })
        .unwrap()
    }

    #[test]
    fn image_lasts_until_the_next_image_or_the_end_of_the_timeline() {
        let project = three_dialogues(vec![
            image(1, "assets/image/a.png", None),
            image(3, "assets/image/b.png", None),
        ]);
        let spans: Vec<(&str, u64, u64)> = project
            .image_clips()
            .iter()
            .map(|c| (c.id.as_str(), c.start_ms, c.duration_ms))
            .collect();
        assert_eq!(
            spans,
            vec![("image-001", 0, 2400), ("image-002", 2400, 1000)]
        );
    }

    #[test]
    fn explicit_duration_wins_but_never_passes_the_end_of_the_timeline() {
        let project = three_dialogues(vec![
            image(1, "assets/image/a.png", Some(500)),
            image(3, "assets/image/b.png", Some(99_000)),
        ]);
        let spans: Vec<(u64, u64)> = project
            .image_clips()
            .iter()
            .map(|c| (c.start_ms, c.duration_ms))
            .collect();
        assert_eq!(spans, vec![(0, 500), (2400, 1000)]);
        assert_eq!(project.total_duration_ms(), 3400);
    }

    #[test]
    fn character_lasts_until_the_same_speakers_next_stand_in() {
        let project = three_dialogues(vec![
            character(1, "reimu", "assets/character/reimu/default.png"),
            character(2, "marisa", "assets/character/marisa/default.png"),
            character(3, "reimu", "assets/character/reimu/happy.png"),
        ]);
        let spans: Vec<(Option<&str>, u64, u64)> = project
            .character_clips()
            .iter()
            .map(|c| (c.speaker.as_deref(), c.start_ms, c.duration_ms))
            .collect();
        assert_eq!(
            spans,
            vec![
                (Some("reimu"), 0, 2400),
                (Some("marisa"), 1200, 2200),
                (Some("reimu"), 2400, 1000),
            ]
        );
    }

    #[test]
    fn bgm_runs_to_the_end_and_sound_effects_get_the_default_length() {
        let project = three_dialogues(vec![
            VisualEvent {
                anchor_dialogue_index: 1,
                duration_ms: None,
                kind: VisualEventKind::Bgm {
                    source: asset("assets/bgm/main.mp3"),
                    volume: 0.6,
                    looping: true,
                },
            },
            VisualEvent {
                anchor_dialogue_index: 2,
                duration_ms: None,
                kind: VisualEventKind::SoundEffect {
                    source: asset("assets/se/pop.wav"),
                    volume: 1.0,
                },
            },
            VisualEvent {
                anchor_dialogue_index: 3,
                duration_ms: None,
                kind: VisualEventKind::SoundEffect {
                    source: asset("assets/se/pop.wav"),
                    volume: 1.0,
                },
            },
        ]);
        let bgm = project.bgm_clips();
        assert_eq!((bgm[0].start_ms, bgm[0].duration_ms), (0, 3400));
        assert!(bgm[0].looping);
        let se: Vec<(u64, u64)> = project
            .sound_effect_clips()
            .iter()
            .map(|c| (c.start_ms, c.duration_ms))
            .collect();
        assert_eq!(se, vec![(1200, DEFAULT_SOUND_EFFECT_MS), (2400, 1000)]);
        let kinds: Vec<TrackKind> = project.tracks.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TrackKind::Audio,
                TrackKind::Caption,
                TrackKind::Bgm,
                TrackKind::SoundEffect
            ],
            "only non-empty visual tracks are added, after audio/caption"
        );
    }

    #[test]
    fn unknown_anchor_is_an_error() {
        let err = build(TimelineInput {
            id: "t".into(),
            title: "T".into(),
            video: VideoSettings::default(),
            source: SourceInfo::default(),
            dialogues: vec![dialogue(1, "reimu", 1000)],
            background: None,
            visual_events: vec![image(7, "assets/image/a.png", None)],
            options: TimelineOptions::default(),
        })
        .unwrap_err();
        assert!(matches!(err, TimelineError::UnknownAnchor { index: 7 }));
    }
}
