//! Contract Test A (P0 §15): freezes the on-disk shape of `project.vfp.json`.
//!
//! This builds one `VideoProject` that exercises every clip kind, then
//! compares its serialized JSON byte-for-byte against a checked-in golden
//! fixture. An unintended change to field names, ordering, or `#[serde]`
//! attributes anywhere in the IR shows up here as a diff against
//! `fixtures/video_project/basic.vfp.json`, even if every crate's own inline
//! assertions still pass.
//!
//! A deliberate, intentional IR change updates the fixture file itself in
//! the same commit — this test is a change detector, not a behavior lock.

use std::collections::BTreeMap;

use videoforge_project::{
    AudioClip, BackgroundClip, BgmClip, CaptionClip, CharacterClip, CharacterPerformanceClip, Clip,
    CropRect, FitMode, ImageClip, Presentation, RelativeAssetPath, SoundEffectClip, SourceInfo,
    TextClip, Track, TrackKind, Transform, VideoClip, VideoProject, VideoSettings,
};

const FIXTURE: &str = include_str!("../../../fixtures/video_project/basic.vfp.json");

/// One project touching every `Clip` variant and every optional field at
/// least once (a populated `Transform`, `Presentation`, `extra`, per-speaker
/// caption color, and a video crop) — the shape this contract freezes.
fn contract_project() -> VideoProject {
    let mut p = VideoProject::new(
        "contract-basic",
        "IR Contract Fixture",
        VideoSettings {
            width: 1280,
            height: 720,
            fps: 30,
        },
    );
    p.source = SourceInfo {
        script: Some("scripts/contract-basic.md".into()),
        template: Some("default".into()),
        generator_version: Some("0.0.0-contract".into()),
    };

    p.tracks.push(Track {
        id: "audio".into(),
        kind: TrackKind::Audio,
        clips: vec![Clip::Audio(AudioClip {
            id: "audio-001".into(),
            source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
            start_ms: 0,
            duration_ms: 900,
            speaker: "reimu".into(),
            extra: BTreeMap::new(),
        })],
    });
    p.tracks.push(Track {
        id: "caption".into(),
        kind: TrackKind::Caption,
        clips: vec![Clip::Caption(CaptionClip {
            id: "caption-001".into(),
            text: "こんにちは。".into(),
            start_ms: 0,
            duration_ms: 900,
            speaker: "reimu".into(),
            speaker_display: Some("霊夢".into()),
            color: Some("#ff66aa".into()),
            extra: BTreeMap::new(),
        })],
    });
    p.tracks.push(Track {
        id: "background".into(),
        kind: TrackKind::Background,
        clips: vec![Clip::Background(BackgroundClip {
            id: "background-001".into(),
            source: RelativeAssetPath::new("assets/image/bg.png").unwrap(),
            start_ms: 0,
            duration_ms: 900,
            extra: BTreeMap::new(),
        })],
    });
    p.tracks.push(Track {
        id: "image".into(),
        kind: TrackKind::Image,
        clips: vec![Clip::Image(ImageClip {
            id: "image-001".into(),
            source: RelativeAssetPath::new("assets/image/diagram.png").unwrap(),
            start_ms: 100,
            duration_ms: 600,
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
                role: Some("diagram".into()),
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
            duration_ms: 900,
            speaker: Some("reimu".into()),
            transform: Transform::default(),
            presentation: None,
            extra: BTreeMap::new(),
        })],
    });
    p.tracks.push(Track {
        id: "character_performance".into(),
        kind: TrackKind::CharacterPerformance,
        clips: vec![Clip::CharacterPerformance(CharacterPerformanceClip {
            id: "character-performance-001".into(),
            start_ms: 0,
            duration_ms: 900,
            character: "tsumugi".into(),
            expression: "smile".into(),
            motion: "wave".into(),
            lip_sync: RelativeAssetPath::new("assets/character/tsumugi/lipsync-001.json").unwrap(),
            transform: Transform::default(),
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
            duration_ms: 900,
            volume: 0.6,
            looping: true,
            trim_start_ms: 0,
            fade_in_ms: 200,
            fade_out_ms: 200,
            normalize: true,
            extra: BTreeMap::new(),
        })],
    });
    p.tracks.push(Track {
        id: "se".into(),
        kind: TrackKind::SoundEffect,
        clips: vec![Clip::SoundEffect(SoundEffectClip {
            id: "se-001".into(),
            source: RelativeAssetPath::new("assets/se/pop.wav").unwrap(),
            start_ms: 200,
            duration_ms: 300,
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
            duration_ms: 900,
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
            duration_ms: 500,
            transform: Transform::default(),
            color: Some("#ffcc00".into()),
            presentation: None,
            extra: BTreeMap::new(),
        })],
    });

    p
}

#[test]
fn video_project_json_matches_the_frozen_contract_fixture() {
    let project = contract_project();
    let actual = project.to_json().unwrap();
    assert_eq!(
        actual, FIXTURE,
        "project.vfp.json shape changed — if this is an intentional, \
         additive change, regenerate fixtures/video_project/basic.vfp.json \
         from `contract_project()`'s output; if not, an accidental change \
         to the IR just escaped every other test"
    );
}

#[test]
fn frozen_contract_fixture_deserializes_back_to_the_same_project() {
    let from_fixture = VideoProject::from_json(FIXTURE).unwrap();
    assert_eq!(from_fixture, contract_project());
}
