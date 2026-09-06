//! Content-addressed WAV cache.
//!
//! Key = SHA256(engine + speaker_id + text + speed + pitch + intonation + volume).
//! Lives in the OS cache directory, never inside the workspace.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::VoiceParams;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct TtsCache {
    dir: PathBuf,
}

impl TtsCache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<platform cache dir>/tts`
    pub fn in_platform_cache(platform_cache_dir: &Path) -> Self {
        Self::new(platform_cache_dir.join("tts"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(engine: &str, text: &str, voice: &VoiceParams) -> String {
        let mut hasher = Sha256::new();
        hasher.update(engine.as_bytes());
        hasher.update([0]);
        hasher.update(voice.speaker_id.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(text.as_bytes());
        hasher.update([0]);
        hasher.update(format!("{:.4}", voice.speed_scale).as_bytes());
        hasher.update([0]);
        hasher.update(format!("{:.4}", voice.pitch_scale).as_bytes());
        hasher.update([0]);
        hasher.update(format!("{:.4}", voice.intonation_scale).as_bytes());
        hasher.update([0]);
        hasher.update(format!("{:.4}", voice.volume_scale).as_bytes());
        hex::encode(hasher.finalize())
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(&key[..2]).join(format!("{key}.wav"))
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path_for(key);
        std::fs::read(&path).ok().filter(|b| !b.is_empty())
    }

    pub fn put(&self, key: &str, wav: &[u8]) -> Result<PathBuf, AppError> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
        }
        let tmp = path.with_extension("wav.part");
        std::fs::write(&tmp, wav).map_err(|e| AppError::write(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| AppError::write(&path, e))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_changes_with_inputs() {
        let v = VoiceParams {
            speaker_id: 2,
            ..Default::default()
        };
        let a = TtsCache::key("voicevox", "こんにちは", &v);
        let b = TtsCache::key("voicevox", "こんにちは。", &v);
        let c = TtsCache::key(
            "voicevox",
            "こんにちは",
            &VoiceParams {
                speed_scale: 1.1,
                ..v
            },
        );
        let d = TtsCache::key("fake", "こんにちは", &v);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, TtsCache::key("voicevox", "こんにちは", &v));
    }

    #[test]
    fn put_then_get() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TtsCache::new(dir.path());
        let key = TtsCache::key("x", "y", &VoiceParams::default());
        assert!(cache.get(&key).is_none());
        cache.put(&key, b"RIFFdata").unwrap();
        assert_eq!(cache.get(&key).unwrap(), b"RIFFdata");
    }
}
