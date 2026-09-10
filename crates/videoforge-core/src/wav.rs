//! Minimal RIFF/WAVE header reader (enough for duration) and a silent WAV
//! generator used by the fake TTS engine.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub data_len: u32,
    pub duration_ms: u64,
}

/// Header fields plus the byte offset of the `data` chunk body, so callers
/// that need the samples (not just the duration) don't have to re-walk the
/// chunk list.
struct ParsedWav {
    info: WavInfo,
    data_start: usize,
}

fn parse_wav(bytes: &[u8]) -> Result<ParsedWav, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u32, u16, u32)> = None; // channels, rate, bits, byte_rate
    let mut data: Option<(usize, u32)> = None; // (start, len)
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]);
        let body_start = pos + 8;
        match id {
            b"fmt " => {
                if body_start + 16 > bytes.len() {
                    return Err("truncated fmt chunk".into());
                }
                let b = &bytes[body_start..];
                let channels = u16::from_le_bytes([b[2], b[3]]);
                let rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                let byte_rate = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
                let bits = u16::from_le_bytes([b[14], b[15]]);
                fmt = Some((channels, rate, bits, byte_rate));
            }
            b"data" => {
                // Some encoders write 0 / 0xFFFFFFFF for streamed output; fall back to
                // the remaining length.
                let remaining = (bytes.len() - body_start) as u32;
                let len = if size == 0 || size == u32::MAX || size > remaining {
                    remaining
                } else {
                    size
                };
                data = Some((body_start, len));
                break;
            }
            _ => {}
        }
        // chunks are word aligned
        pos = body_start + size as usize + (size as usize & 1);
    }
    let (channels, sample_rate, bits_per_sample, byte_rate) = fmt.ok_or("missing fmt chunk")?;
    let (data_start, data_len) = data.ok_or("missing data chunk")?;
    let byte_rate = if byte_rate == 0 {
        sample_rate * channels as u32 * bits_per_sample as u32 / 8
    } else {
        byte_rate
    };
    if byte_rate == 0 {
        return Err("invalid byte rate".into());
    }
    let duration_ms = (data_len as u64 * 1000 + byte_rate as u64 / 2) / byte_rate as u64;
    Ok(ParsedWav {
        info: WavInfo {
            sample_rate,
            channels,
            bits_per_sample,
            data_len,
            duration_ms,
        },
        data_start,
    })
}

pub fn parse_wav_info(bytes: &[u8]) -> Result<WavInfo, String> {
    parse_wav(bytes).map(|p| p.info)
}

/// Decode a 16-bit PCM RIFF/WAVE into mono samples normalized to `[-1.0, 1.0]`.
/// Multi-channel input is downmixed by averaging channels. Used for lip-sync
/// amplitude analysis (`crate::lipsync`), never for playback, so no other bit
/// depth is supported yet — VOICEVOX and the fake engine both emit 16-bit PCM.
pub fn decode_pcm16_mono(bytes: &[u8]) -> Result<(WavInfo, Vec<f32>), String> {
    let parsed = parse_wav(bytes)?;
    let info = parsed.info;
    if info.bits_per_sample != 16 {
        return Err(format!(
            "unsupported bits_per_sample {} (only 16-bit PCM is supported)",
            info.bits_per_sample
        ));
    }
    let data = &bytes[parsed.data_start..parsed.data_start + info.data_len as usize];
    let channels = info.channels.max(1) as usize;
    let frame_bytes = 2 * channels;
    let mut samples = Vec::with_capacity(data.len() / frame_bytes.max(1));
    for frame in data.chunks_exact(frame_bytes) {
        let mut sum = 0i32;
        for ch in frame.chunks_exact(2) {
            sum += i16::from_le_bytes([ch[0], ch[1]]) as i32;
        }
        let avg = sum as f32 / channels as f32;
        samples.push(avg / 32768.0);
    }
    Ok((info, samples))
}

/// 16-bit mono PCM silence of the given length.
pub fn silent_wav(duration_ms: u64, sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * block_align as u32;
    let samples = (sample_rate as u64 * duration_ms / 1000) as u32;
    let data_len = samples * block_align as u32;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_wav_roundtrips_duration() {
        let wav = silent_wav(3410, 24000);
        let info = parse_wav_info(&wav).unwrap();
        assert_eq!(info.sample_rate, 24000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.duration_ms, 3410);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_wav_info(b"hello").is_err());
        assert!(parse_wav_info(b"RIFF\0\0\0\0WAVEjunk").is_err());
    }

    #[test]
    fn handles_extra_chunks_before_data() {
        let mut wav = silent_wav(1000, 8000);
        // insert a LIST chunk after fmt (offset 36)
        let list: Vec<u8> = [b"LIST".as_slice(), &4u32.to_le_bytes(), b"INFO"].concat();
        wav.splice(36..36, list);
        let info = parse_wav_info(&wav).unwrap();
        assert_eq!(info.duration_ms, 1000);
    }

    #[test]
    fn decodes_silence_to_all_zero_samples() {
        let wav = silent_wav(100, 8000);
        let (info, samples) = decode_pcm16_mono(&wav).unwrap();
        assert_eq!(samples.len(), 800);
        assert!(samples.iter().all(|s| *s == 0.0), "{info:?}");
    }

    #[test]
    fn decodes_known_pcm_values() {
        // Hand-built mono 16-bit WAV with two frames: full-scale positive then
        // full-scale negative.
        let mut wav = silent_wav(0, 8000);
        wav.truncate(44); // header only, no data bytes yet
        let data: [i16; 2] = [i16::MAX, i16::MIN];
        let mut data_bytes = Vec::new();
        for s in data {
            data_bytes.extend_from_slice(&s.to_le_bytes());
        }
        // patch RIFF/data sizes for the 4 bytes of payload we're appending.
        let data_len = data_bytes.len() as u32;
        wav[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
        wav[40..44].copy_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&data_bytes);

        let (_, samples) = decode_pcm16_mono(&wav).unwrap();
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 1.0).abs() < 0.001);
        assert!((samples[1] + 1.0).abs() < 0.001);
    }

    #[test]
    fn decodes_and_downmixes_stereo() {
        let channels: u16 = 2;
        let bits: u16 = 16;
        let sample_rate = 8000u32;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * block_align as u32;
        let mut wav = Vec::new();
        let left = 10_000i16;
        let right = -10_000i16;
        let data_len = 4u32; // one stereo frame
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&left.to_le_bytes());
        wav.extend_from_slice(&right.to_le_bytes());

        let (info, samples) = decode_pcm16_mono(&wav).unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(samples.len(), 1);
        assert!(samples[0].abs() < 0.0001, "left+right should cancel out");
    }
}
