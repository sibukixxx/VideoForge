//! TTS engine abstraction (design §13), cache (§16) and batch synthesis.

pub mod cache;
pub mod fake;
pub mod synth;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::VoiceParams;
use crate::error::AppError;

pub use cache::{bind_cache, BoundCache, TtsCache, CACHE_SCHEMA_VERSION};
pub use fake::FakeTtsEngine;
pub use synth::{synthesize_all, SynthesisJob, SynthesizedDialogue};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub engine: String,
    pub version: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerStyle {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    pub name: String,
    pub styles: Vec<SpeakerStyle>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TtsRequest {
    pub text: String,
    pub voice: VoiceParams,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedAudio {
    /// Complete WAV file bytes.
    pub wav: Vec<u8>,
}

#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Stable engine id, part of the cache key (`voicevox`, `fake`).
    fn id(&self) -> &str;

    /// Reachability / version check.
    async fn health(&self) -> Result<EngineInfo, AppError>;

    async fn list_speakers(&self) -> Result<Vec<Speaker>, AppError>;

    async fn synthesize(&self, request: &TtsRequest) -> Result<SynthesizedAudio, AppError>;
}
