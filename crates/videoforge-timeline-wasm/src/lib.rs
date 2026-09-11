//! Thin JSON boundary around the existing pure timeline scheduler.
//!
//! Business logic remains in `videoforge-timeline`; this crate only decodes,
//! delegates, and encodes so native and browser runtimes cannot drift.

use serde::{Deserialize, Serialize};
use videoforge_project::{RelativeAssetPath, Track};
use videoforge_timeline::{
    place_visual_events, schedule, DialogueInput, ScheduledDialogue, TimelineOptions, VisualEvent,
};

#[derive(Debug, Deserialize)]
struct ScheduleRequest {
    #[serde(default = "default_dialogue_gap_ms")]
    dialogue_gap_ms: u64,
    dialogues: Vec<DialogueRequest>,
    #[serde(default)]
    visual_events: Vec<VisualEvent>,
}

#[derive(Debug, Deserialize)]
struct DialogueRequest {
    index: usize,
    speaker: String,
    speaker_display: String,
    text: String,
    audio: String,
    duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct ScheduleResponse {
    dialogues: Vec<ScheduledDialogue>,
    total_duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct VisualTimelineResponse {
    dialogues: Vec<ScheduledDialogue>,
    visual_tracks: Vec<Track>,
    total_duration_ms: u64,
}

const fn default_dialogue_gap_ms() -> u64 {
    200
}

/// Native-testable implementation of the Wasm JSON contract.
pub fn calculate_timeline_json(input: &str) -> Result<String, String> {
    let request: ScheduleRequest =
        serde_json::from_str(input).map_err(|error| format!("invalid input JSON: {error}"))?;
    let dialogues = decode_dialogues(request.dialogues)?;

    let result = schedule(
        &dialogues,
        &TimelineOptions {
            dialogue_gap_ms: request.dialogue_gap_ms,
        },
    )
    .map_err(|error| error.to_string())?;
    let total_duration_ms = result.last().map_or(0, |dialogue| dialogue.end_ms);
    serde_json::to_string(&ScheduleResponse {
        dialogues: result,
        total_duration_ms,
    })
    .map_err(|error| format!("failed to encode result: {error}"))
}

/// Schedule dialogues and place visual clips in one boundary crossing.
pub fn calculate_visual_timeline_json(input: &str) -> Result<String, String> {
    let request: ScheduleRequest =
        serde_json::from_str(input).map_err(|error| format!("invalid input JSON: {error}"))?;
    let dialogues = decode_dialogues(request.dialogues)?;
    let scheduled = schedule(
        &dialogues,
        &TimelineOptions {
            dialogue_gap_ms: request.dialogue_gap_ms,
        },
    )
    .map_err(|error| error.to_string())?;
    let total_duration_ms = scheduled.last().map_or(0, |dialogue| dialogue.end_ms);
    let visual_tracks = place_visual_events(&request.visual_events, &scheduled, total_duration_ms)
        .map_err(|error| error.to_string())?;

    serde_json::to_string(&VisualTimelineResponse {
        dialogues: scheduled,
        visual_tracks,
        total_duration_ms,
    })
    .map_err(|error| format!("failed to encode result: {error}"))
}

fn decode_dialogues(dialogues: Vec<DialogueRequest>) -> Result<Vec<DialogueInput>, String> {
    dialogues
        .into_iter()
        .map(|dialogue| {
            let audio = RelativeAssetPath::new(dialogue.audio)
                .map_err(|error| format!("invalid audio path: {error}"))?;
            Ok(DialogueInput {
                index: dialogue.index,
                speaker: dialogue.speaker,
                speaker_display: dialogue.speaker_display,
                text: dialogue.text,
                audio,
                duration_ms: dialogue.duration_ms,
            })
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;

    /// Calculate dialogue placement from a small JSON payload.
    #[wasm_bindgen]
    pub fn calculate_timeline(input: &str) -> Result<String, JsValue> {
        super::calculate_timeline_json(input).map_err(|error| JsValue::from_str(&error))
    }

    /// Calculate dialogue and visual placement in a single Wasm call.
    #[wasm_bindgen]
    pub fn calculate_visual_timeline(input: &str) -> Result<String, JsValue> {
        super::calculate_visual_timeline_json(input).map_err(|error| JsValue::from_str(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_match_native_scheduler_and_expected_results() {
        let fixtures = [
            (
                "simple",
                include_str!("../../../fixtures/micro-wasm/simple.input.json"),
                include_str!("../../../fixtures/micro-wasm/simple.expected.json"),
            ),
            (
                "multi-clip",
                include_str!("../../../fixtures/micro-wasm/multi-clip.input.json"),
                include_str!("../../../fixtures/micro-wasm/multi-clip.expected.json"),
            ),
            (
                "edge-case",
                include_str!("../../../fixtures/micro-wasm/edge-case.input.json"),
                include_str!("../../../fixtures/micro-wasm/edge-case.expected.json"),
            ),
        ];
        for (name, input, expected) in fixtures {
            let actual: serde_json::Value =
                serde_json::from_str(&calculate_timeline_json(input).unwrap()).unwrap();
            let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
            assert_eq!(actual, expected, "fixture {name}");
        }
    }

    #[test]
    fn rejects_zero_duration_through_the_adapter() {
        let input = r#"{
            "dialogues": [
                {"index": 1, "speaker": "x", "speaker_display": "X", "text": "x", "audio": "assets/audio/x.wav", "duration_ms": 0}
            ]
        }"#;
        assert!(calculate_timeline_json(input)
            .unwrap_err()
            .contains("zero duration"));
    }

    #[test]
    fn visual_fixture_matches_native_placement() {
        let input = include_str!("../../../fixtures/micro-wasm/visual-placement.input.json");
        let expected = include_str!("../../../fixtures/micro-wasm/visual-placement.expected.json");
        let actual: serde_json::Value =
            serde_json::from_str(&calculate_visual_timeline_json(input).unwrap()).unwrap();
        let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn visual_adapter_rejects_an_unknown_dialogue_anchor() {
        let input = r#"{
            "dialogues": [
                {"index": 1, "speaker": "x", "speaker_display": "X", "text": "x", "audio": "assets/audio/x.wav", "duration_ms": 100}
            ],
            "visual_events": [
                {"anchor_dialogue_index": 9, "duration_ms": null, "kind": {"type": "image", "source": "assets/image/x.png", "transform": {}, "presentation": null}}
            ]
        }"#;
        assert!(calculate_visual_timeline_json(input)
            .unwrap_err()
            .contains("does not exist"));
    }
}
