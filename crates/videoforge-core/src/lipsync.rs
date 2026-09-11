//! Deterministic WAV → lip-sync amplitude curve (design §10).
//!
//! P0 does not do phoneme analysis: a mouth-open value is derived purely
//! from RMS amplitude in fixed-size time windows. It is a real timeline
//! artifact — written to disk, referenced by a `CharacterPerformanceClip`,
//! and reproducible from the same WAV, not something computed only at
//! playback time — so a renderer built later (Phase 1, see
//! `docs/live2d-renderer-decision.md`) can render frames offline from it.

use serde::{Deserialize, Serialize};

use crate::wav::decode_pcm16_mono;

/// Default sampling interval for a lip-sync curve, matching the example in
/// design §10 (0ms, 50ms, 100ms, ...).
pub const DEFAULT_INTERVAL_MS: u32 = 50;

/// One sample of a lip-sync curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LipSyncSample {
    pub t_ms: u32,
    /// Normalized mouth-open value, `0.0` (closed) .. `1.0` (fully open).
    pub mouth_open: f32,
}

/// A deterministic, offline-renderable lip-sync curve for one dialogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LipSyncTrack {
    pub interval_ms: u32,
    pub samples: Vec<LipSyncSample>,
}

impl LipSyncTrack {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// RMS amplitude in fixed-size windows of `interval_ms`, mapped to a
/// `mouth_open` value. `GAIN` boosts quiet dialogue so it still opens the
/// mouth a visible amount; it is tuned against VOICEVOX's typical loudness,
/// not a perceptual model — good enough for P0's "does the mouth move with
/// the voice" bar, not a substitute for real phoneme analysis (design §10).
const GAIN: f32 = 4.0;

/// Analyze `wav_bytes` (16-bit PCM RIFF/WAVE) into a lip-sync curve sampled
/// every `interval_ms`. Pure and deterministic: the same WAV always produces
/// the same curve.
pub fn analyze_amplitude(wav_bytes: &[u8], interval_ms: u32) -> Result<LipSyncTrack, String> {
    if interval_ms == 0 {
        return Err("interval_ms must be greater than 0".into());
    }
    let (info, pcm) = decode_pcm16_mono(wav_bytes)?;
    let window_len = ((info.sample_rate as u64 * interval_ms as u64) / 1000).max(1) as usize;

    let mut samples = Vec::new();
    if pcm.is_empty() {
        samples.push(LipSyncSample {
            t_ms: 0,
            mouth_open: 0.0,
        });
        return Ok(LipSyncTrack {
            interval_ms,
            samples,
        });
    }
    for (i, window) in pcm.chunks(window_len).enumerate() {
        let sum_sq: f32 = window.iter().map(|s| s * s).sum();
        let rms = (sum_sq / window.len() as f32).sqrt();
        let mouth_open = (rms * GAIN).clamp(0.0, 1.0);
        samples.push(LipSyncSample {
            t_ms: (i as u64 * interval_ms as u64) as u32,
            mouth_open,
        });
    }
    Ok(LipSyncTrack {
        interval_ms,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav::silent_wav;

    #[test]
    fn silence_is_all_closed_mouth() {
        let wav = silent_wav(500, 24000);
        let track = analyze_amplitude(&wav, 50).unwrap();
        assert_eq!(track.interval_ms, 50);
        assert_eq!(track.samples.len(), 10);
        assert!(track.samples.iter().all(|s| s.mouth_open == 0.0));
        assert_eq!(track.samples[3].t_ms, 150);
    }

    #[test]
    fn loud_tone_opens_the_mouth() {
        // 100ms of a full-scale-ish square wave at 24kHz.
        let sample_rate = 24000u32;
        let n = (sample_rate / 10) as usize; // 100ms
        let wav = build_tone_wav(sample_rate, n, i16::MAX / 2);
        let track = analyze_amplitude(&wav, 50).unwrap();
        assert_eq!(track.samples.len(), 2);
        assert!(
            track.samples.iter().all(|s| s.mouth_open > 0.3),
            "{track:?}"
        );
    }

    #[test]
    fn rejects_zero_interval() {
        let wav = silent_wav(10, 8000);
        assert!(analyze_amplitude(&wav, 0).is_err());
    }

    #[test]
    fn same_input_produces_the_same_curve() {
        let wav = build_tone_wav(24000, 2400, 12000);
        let a = analyze_amplitude(&wav, 50).unwrap();
        let b = analyze_amplitude(&wav, 50).unwrap();
        assert_eq!(a, b);
    }

    fn build_tone_wav(sample_rate: u32, n_samples: usize, amplitude: i16) -> Vec<u8> {
        let channels: u16 = 1;
        let bits: u16 = 16;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * block_align as u32;
        let data_len = (n_samples * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..n_samples {
            let v = if i % 2 == 0 { amplitude } else { -amplitude };
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}
