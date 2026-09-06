//! Engine-level integration test against a *real* VOICEVOX Engine (issue #10).
//!
//! Runs only when an engine answers `GET /version` at
//! `$VIDEOFORGE_VOICEVOX_ENDPOINT` (default `http://127.0.0.1:50021`);
//! otherwise it prints `skipped:` and passes, so the offline
//! `cargo test --workspace` stays green. The full CLI pipeline with a real
//! engine lives in `videoforge-cli/tests/voicevox_real.rs`; the manual
//! procedure and results log are in `docs/testing/voicevox-manual-e2e.md`.

use std::time::Duration;

use videoforge_core::config::VoiceParams;
use videoforge_core::tts::{EngineInfo, TtsEngine, TtsRequest};
use videoforge_core::wav::parse_wav_info;
use videoforge_voicevox::{VoicevoxEngine, DEFAULT_ENDPOINT};

async fn connect() -> Option<(VoicevoxEngine, EngineInfo)> {
    let endpoint = std::env::var("VIDEOFORGE_VOICEVOX_ENDPOINT")
        .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
    let engine = VoicevoxEngine::new(&endpoint, Duration::from_secs(120), false)
        .expect("loopback endpoint is always allowed");
    match engine.health().await {
        Ok(info) => Some((engine, info)),
        Err(e) => {
            eprintln!("skipped: no VOICEVOX at {endpoint} ({e}); set VIDEOFORGE_VOICEVOX_ENDPOINT");
            None
        }
    }
}

async fn first_style_id(engine: &VoicevoxEngine) -> u32 {
    engine
        .list_speakers()
        .await
        .expect("GET /speakers")
        .iter()
        .flat_map(|s| s.styles.iter())
        .next()
        .expect("engine reports at least one style")
        .id
}

#[tokio::test]
async fn audio_query_then_synthesis_returns_a_wav_with_positive_duration() {
    let Some((engine, info)) = connect().await else {
        return;
    };
    eprintln!("VOICEVOX {} at {}", info.version, info.endpoint);
    assert!(
        !info.version.trim().is_empty(),
        "version is what keys the TTS cache"
    );
    let style_id = first_style_id(&engine).await;

    let audio = engine
        .synthesize(&TtsRequest {
            text: "こんにちは、テストです。".into(),
            voice: VoiceParams {
                speaker_id: style_id,
                ..VoiceParams::default()
            },
        })
        .await
        .expect("audio_query → synthesis");

    let wav = parse_wav_info(&audio.wav).expect("synthesis returns RIFF/WAVE");
    assert!(wav.duration_ms > 0, "{wav:?}");
    assert!(wav.sample_rate > 0, "{wav:?}");
    assert!(wav.data_len > 0, "{wav:?}");
}

#[tokio::test]
async fn speed_scale_override_shortens_the_audio() {
    let Some((engine, _)) = connect().await else {
        return;
    };
    let style_id = first_style_id(&engine).await;
    let request = |speed_scale: f32| TtsRequest {
        text: "音声パラメータの上書きが効いているか確認します。".into(),
        voice: VoiceParams {
            speaker_id: style_id,
            speed_scale,
            ..VoiceParams::default()
        },
    };

    let normal = engine.synthesize(&request(1.0)).await.unwrap();
    let fast = engine.synthesize(&request(1.5)).await.unwrap();

    let normal_ms = parse_wav_info(&normal.wav).unwrap().duration_ms;
    let fast_ms = parse_wav_info(&fast.wav).unwrap().duration_ms;
    assert!(
        fast_ms < normal_ms,
        "speed_scale 1.5 ({fast_ms} ms) must be shorter than 1.0 ({normal_ms} ms)"
    );
}
