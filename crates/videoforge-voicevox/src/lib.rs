//! VOICEVOX Engine adapter (design §13–§15).
//!
//! ```text
//! audio_query → parameter override → synthesis → WAV
//! ```
//!
//! The engine is reached over HTTP only, so Windows and macOS share this
//! implementation unchanged.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use videoforge_core::config::VoiceParams;
use videoforge_core::tts::{
    EngineInfo, Speaker, SpeakerStyle, SynthesizedAudio, TtsEngine, TtsRequest,
};
use videoforge_core::AppError;

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:50021";

#[derive(Debug, Clone)]
pub struct VoicevoxEngine {
    endpoint: String,
    client: reqwest::Client,
}

impl VoicevoxEngine {
    /// `allow_remote_endpoint` mirrors `videoforge.yaml`'s `tts.allow_remote_endpoint`
    /// (VF-004): by default `endpoint` must be localhost/127.0.0.1/::1. This is
    /// checked again here (not just in `Config::validate`) so that callers who
    /// construct the engine directly with a CLI-supplied endpoint cannot bypass
    /// the restriction. Redirects are disabled so a compromised or misconfigured
    /// endpoint cannot redirect requests elsewhere.
    pub fn new(
        endpoint: impl Into<String>,
        timeout: Duration,
        allow_remote_endpoint: bool,
    ) -> Result<Self, AppError> {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        videoforge_core::config::check_endpoint_allowed(&endpoint, allow_remote_endpoint)?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        Ok(Self { endpoint, client })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn unavailable(&self, reason: impl ToString) -> AppError {
        AppError::VoicevoxUnavailable {
            endpoint: self.endpoint.clone(),
            reason: reason.to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.endpoint, path)
    }

    /// Fetch the `AudioQuery` for `text` and apply the voice overrides.
    async fn audio_query(
        &self,
        text: &str,
        voice: &VoiceParams,
    ) -> Result<serde_json::Value, AppError> {
        let response = self
            .client
            .post(self.url("/audio_query"))
            .query(&[("text", text), ("speaker", &voice.speaker_id.to_string())])
            .send()
            .await
            .map_err(|e| self.unavailable(format!("audio_query: {e}")))?;
        let response = check_status(response, "audio_query").await?;
        let mut query: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Other(format!("audio_query returned invalid JSON: {e}")))?;
        apply_overrides(&mut query, voice);
        Ok(query)
    }
}

fn apply_overrides(query: &mut serde_json::Value, voice: &VoiceParams) {
    if let Some(obj) = query.as_object_mut() {
        obj.insert("speedScale".into(), json_f32(voice.speed_scale));
        obj.insert("pitchScale".into(), json_f32(voice.pitch_scale));
        obj.insert("intonationScale".into(), json_f32(voice.intonation_scale));
        obj.insert("volumeScale".into(), json_f32(voice.volume_scale));
    }
}

fn json_f32(v: f32) -> serde_json::Value {
    serde_json::Number::from_f64(v as f64)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

async fn check_status(
    response: reqwest::Response,
    what: &str,
) -> Result<reqwest::Response, AppError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let body = body.chars().take(300).collect::<String>();
    Err(AppError::Other(format!(
        "{what} failed with HTTP {status}: {body}"
    )))
}

#[derive(Debug, Deserialize)]
struct ApiSpeaker {
    name: String,
    #[serde(default)]
    styles: Vec<ApiStyle>,
}

#[derive(Debug, Deserialize)]
struct ApiStyle {
    id: u32,
    name: String,
}

#[async_trait]
impl TtsEngine for VoicevoxEngine {
    fn id(&self) -> &str {
        "voicevox"
    }

    async fn health(&self) -> Result<EngineInfo, AppError> {
        let response = self
            .client
            .get(self.url("/version"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| self.unavailable(e))?;
        let response = check_status(response, "version")
            .await
            .map_err(|e| self.unavailable(e))?;
        let version: serde_json::Value = response.json().await.map_err(|e| self.unavailable(e))?;
        let version = version
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| version.to_string());
        Ok(EngineInfo {
            engine: "VOICEVOX".into(),
            version,
            endpoint: self.endpoint.clone(),
        })
    }

    async fn list_speakers(&self) -> Result<Vec<Speaker>, AppError> {
        let response = self
            .client
            .get(self.url("/speakers"))
            .send()
            .await
            .map_err(|e| self.unavailable(e))?;
        let response = check_status(response, "speakers").await?;
        let speakers: Vec<ApiSpeaker> = response
            .json()
            .await
            .map_err(|e| AppError::Other(format!("speakers returned invalid JSON: {e}")))?;
        Ok(speakers
            .into_iter()
            .map(|s| Speaker {
                name: s.name,
                styles: s
                    .styles
                    .into_iter()
                    .map(|st| SpeakerStyle {
                        id: st.id,
                        name: st.name,
                    })
                    .collect(),
            })
            .collect())
    }

    async fn synthesize(&self, request: &TtsRequest) -> Result<SynthesizedAudio, AppError> {
        let query = self.audio_query(&request.text, &request.voice).await?;
        let response = self
            .client
            .post(self.url("/synthesis"))
            .query(&[("speaker", request.voice.speaker_id.to_string())])
            .json(&query)
            .send()
            .await
            .map_err(|e| self.unavailable(format!("synthesis: {e}")))?;
        let response = check_status(response, "synthesis").await?;
        let wav = response
            .bytes()
            .await
            .map_err(|e| AppError::Other(format!("synthesis body: {e}")))?;
        Ok(SynthesizedAudio { wav: wav.to_vec() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_applied() {
        let mut q = serde_json::json!({"accent_phrases": [], "speedScale": 1.0, "outputSamplingRate": 24000});
        apply_overrides(
            &mut q,
            &VoiceParams {
                speaker_id: 2,
                speed_scale: 1.05,
                pitch_scale: -0.02,
                intonation_scale: 1.2,
                volume_scale: 1.0,
            },
        );
        assert!((q["speedScale"].as_f64().unwrap() - 1.05).abs() < 1e-6);
        assert!((q["pitchScale"].as_f64().unwrap() + 0.02).abs() < 1e-6);
        assert!((q["intonationScale"].as_f64().unwrap() - 1.2).abs() < 1e-6);
        assert_eq!(q["outputSamplingRate"], 24000);
    }

    #[test]
    fn endpoint_trailing_slash_is_trimmed() {
        let e =
            VoicevoxEngine::new("http://localhost:50021/", Duration::from_secs(1), false).unwrap();
        assert_eq!(e.endpoint(), "http://localhost:50021");
        assert_eq!(e.url("/version"), "http://localhost:50021/version");
    }

    #[tokio::test]
    async fn unreachable_endpoint_reports_unavailable() {
        let e = VoicevoxEngine::new("http://127.0.0.1:1", Duration::from_secs(1), false).unwrap();
        let err = e.health().await.unwrap_err();
        assert!(matches!(err, AppError::VoicevoxUnavailable { .. }), "{err}");
    }

    #[test]
    fn remote_endpoint_is_rejected_without_opt_in() {
        let err = VoicevoxEngine::new("http://example.com:50021", Duration::from_secs(1), false)
            .unwrap_err();
        assert!(
            matches!(err, AppError::RemoteEndpointNotAllowed { .. }),
            "{err}"
        );
    }

    #[test]
    fn remote_endpoint_is_allowed_with_opt_in() {
        assert!(
            VoicevoxEngine::new("http://example.com:50021", Duration::from_secs(1), true).is_ok()
        );
    }
}
