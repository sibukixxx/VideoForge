//! Deterministic offline engine for tests and dry runs. Produces silence whose
//! duration is derived from the text length.

use async_trait::async_trait;

use super::{EngineInfo, Speaker, SpeakerStyle, SynthesizedAudio, TtsEngine, TtsRequest};
use crate::error::AppError;
use crate::wav::silent_wav;

#[derive(Debug, Clone)]
pub struct FakeTtsEngine {
    pub ms_per_char: u64,
    pub min_ms: u64,
    pub sample_rate: u32,
}

impl Default for FakeTtsEngine {
    fn default() -> Self {
        Self {
            ms_per_char: 150,
            min_ms: 500,
            sample_rate: 24000,
        }
    }
}

impl FakeTtsEngine {
    pub fn duration_for(&self, text: &str, speed_scale: f32) -> u64 {
        let chars = text.chars().filter(|c| !c.is_whitespace()).count() as u64;
        let base = (chars * self.ms_per_char).max(self.min_ms);
        let speed = if speed_scale > 0.0 {
            speed_scale as f64
        } else {
            1.0
        };
        (base as f64 / speed).round() as u64
    }
}

#[async_trait]
impl TtsEngine for FakeTtsEngine {
    fn id(&self) -> &str {
        "fake"
    }

    async fn health(&self) -> Result<EngineInfo, AppError> {
        Ok(EngineInfo {
            engine: "fake".into(),
            version: "0".into(),
            endpoint: "in-process".into(),
        })
    }

    async fn list_speakers(&self) -> Result<Vec<Speaker>, AppError> {
        Ok(vec![Speaker {
            name: "Fake".into(),
            styles: vec![SpeakerStyle {
                id: 0,
                name: "silence".into(),
            }],
        }])
    }

    async fn synthesize(&self, request: &TtsRequest) -> Result<SynthesizedAudio, AppError> {
        let ms = self.duration_for(&request.text, request.voice.speed_scale);
        Ok(SynthesizedAudio {
            wav: silent_wav(ms, self.sample_rate),
        })
    }
}
