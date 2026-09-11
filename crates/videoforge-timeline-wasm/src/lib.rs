//! Thin JSON boundary around the existing pure timeline scheduler.
//!
//! Business logic remains in `videoforge-timeline`; this crate only decodes,
//! delegates, and encodes so native and browser runtimes cannot drift.

use serde::{Deserialize, Serialize};
use videoforge_project::RelativeAssetPath;
use videoforge_timeline::{schedule, DialogueInput, ScheduledDialogue, TimelineOptions};

#[derive(Debug, Deserialize)]
struct ScheduleRequest {
    #[serde(default = "default_dialogue_gap_ms")]
    dialogue_gap_ms: u64,
    dialogues: Vec<DialogueRequest>,
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

const fn default_dialogue_gap_ms() -> u64 {
    200
}

/// Native-testable implementation of the Wasm JSON contract.
pub fn calculate_timeline_json(input: &str) -> Result<String, String> {
    let request: ScheduleRequest =
        serde_json::from_str(input).map_err(|error| format!("invalid input JSON: {error}"))?;
    let dialogues = request
        .dialogues
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
        .collect::<Result<Vec<_>, String>>()?;

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

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;

    /// Calculate dialogue placement from a small JSON payload.
    #[wasm_bindgen]
    pub fn calculate_timeline(input: &str) -> Result<String, JsValue> {
        super::calculate_timeline_json(input).map_err(|error| JsValue::from_str(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_match_native_scheduler_and_expected_results() {
        for name in ["simple", "multi-clip", "edge-case"] {
            let input = std::fs::read_to_string(format!(
                "../../fixtures/micro-wasm/{name}.input.json"
            ))
            .unwrap();
            let expected = std::fs::read_to_string(format!(
                "../../fixtures/micro-wasm/{name}.expected.json"
            ))
            .unwrap();
            let actual: serde_json::Value =
                serde_json::from_str(&calculate_timeline_json(&input).unwrap()).unwrap();
            let expected: serde_json::Value = serde_json::from_str(&expected).unwrap();
            assert_eq!(actual, expected, "fixture {name}");
        }
    }

    #[test]
    fn rejects_zero_duration_through_the_adapter() {
        let input = r#"{"dialogues":[{"index":1,"speaker":"x","speaker_display":"X","text":"x","audio":"assets/audio/x.wav","duration_ms":0}]}"#;
        assert!(calculate_timeline_json(input)
            .unwrap_err()
            .contains("zero duration"));
    }
}
