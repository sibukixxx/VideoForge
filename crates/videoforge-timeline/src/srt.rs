//! SubRip (`.srt`) export from the caption track.

use videoforge_project::{format_srt_timestamp, VideoProject};

/// Options for SRT rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct SrtOptions {
    /// Prefix each cue with the speaker display name (`霊夢: ...`).
    pub include_speaker: bool,
}

pub fn render(project: &VideoProject, options: SrtOptions) -> String {
    let mut out = String::new();
    for (n, cap) in project.caption_clips().iter().enumerate() {
        let start = format_srt_timestamp(cap.start_ms);
        let end = format_srt_timestamp(cap.start_ms + cap.duration_ms);
        out.push_str(&format!("{}\n{start} --> {end}\n", n + 1));
        if options.include_speaker {
            let name = cap.speaker_display.as_deref().unwrap_or(&cap.speaker);
            out.push_str(&format!("{name}: "));
        }
        out.push_str(&cap.text.replace("\r\n", "\n"));
        out.push_str("\n\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use videoforge_project::{CaptionClip, Clip, Track, TrackKind, VideoSettings};

    #[test]
    fn renders_cues() {
        let mut project = VideoProject::new("x", "X", VideoSettings::default());
        project.tracks.push(Track {
            id: "caption".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "c1".into(),
                    text: "こんにちは".into(),
                    start_ms: 0,
                    duration_ms: 3410,
                    speaker: "reimu".into(),
                    speaker_display: Some("霊夢".into()),
                    extra: BTreeMap::new(),
                }),
                Clip::Caption(CaptionClip {
                    id: "c2".into(),
                    text: "やあ\n二行目".into(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    speaker_display: None,
                    extra: BTreeMap::new(),
                }),
            ],
        });
        let srt = render(&project, SrtOptions::default());
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:03,410\nこんにちは\n\n2\n00:00:03,610 --> 00:00:07,290\nやあ\n二行目\n\n"
        );
        let srt = render(
            &project,
            SrtOptions {
                include_speaker: true,
            },
        );
        assert!(srt.contains("霊夢: こんにちは"));
        assert!(srt.contains("marisa: やあ"));
    }
}
