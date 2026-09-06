//! Content-addressed WAV cache (design §16).
//!
//! Key = SHA256(schema, engine id, engine version, speaker_id, text, speed,
//! pitch, intonation, volume). Lives in the OS cache directory, never inside
//! the workspace.
//!
//! # Engine version
//!
//! The engine version is part of the key so that upgrading VOICEVOX (whose
//! output for the same input changes between releases) is a cache miss rather
//! than a silent replay of audio from the previous build. The version comes
//! from [`TtsEngine::health`](super::TtsEngine::health) via [`bind_cache`];
//! when it cannot be obtained the cache is **not used at all for that run**
//! (neither read nor written) and the caller reports a warning. Guessing a
//! version, or falling back to a version-less key, would reintroduce exactly
//! the stale-audio problem this key exists to prevent.
//!
//! # Key schema
//!
//! [`CACHE_SCHEMA_VERSION`] is hashed into every key *and* used as a
//! directory level (`<dir>/v<N>/<xx>/<key>.wav`), so a change to what the key
//! covers never collides with entries written by an older binary. Bump it
//! whenever the set of hashed inputs changes. Entries from an older schema are
//! simply orphaned under their own `v<N>` directory and can be deleted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::TtsEngine;
use crate::config::VoiceParams;
use crate::error::AppError;

/// Bumped whenever the inputs hashed by [`TtsCache::key`] change.
///
/// * 1 — engine id, speaker, text, voice parameters (no directory level).
/// * 2 — adds the engine version; entries live under `v2/`.
pub const CACHE_SCHEMA_VERSION: u32 = 2;

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

    /// Root of the cache (all schema versions).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Directory holding entries for the current key schema.
    pub fn schema_dir(&self) -> PathBuf {
        self.dir.join(format!("v{CACHE_SCHEMA_VERSION}"))
    }

    pub fn key(engine: &str, engine_version: &str, text: &str, voice: &VoiceParams) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("videoforge-tts-cache/{CACHE_SCHEMA_VERSION}").as_bytes());
        hasher.update([0]);
        hasher.update(engine.as_bytes());
        hasher.update([0]);
        hasher.update(engine_version.as_bytes());
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
        self.schema_dir().join(&key[..2]).join(format!("{key}.wav"))
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

/// A [`TtsCache`] bound to one engine build: the engine id and version that
/// go into every key. Obtain one with [`bind_cache`].
#[derive(Debug, Clone)]
pub struct BoundCache {
    pub cache: Arc<TtsCache>,
    pub engine_id: String,
    pub engine_version: String,
}

impl BoundCache {
    pub fn key(&self, text: &str, voice: &VoiceParams) -> String {
        TtsCache::key(&self.engine_id, &self.engine_version, text, voice)
    }
}

/// Resolve the engine's version (via `health()`) and bind `cache` to it.
///
/// Fails when the version cannot be determined. Callers are expected to run
/// **without** a cache in that case and surface the error as a warning — see
/// the module docs for why a version-less key is not an option.
pub async fn bind_cache(
    engine: &dyn TtsEngine,
    cache: Arc<TtsCache>,
) -> Result<BoundCache, AppError> {
    let info = engine.health().await?;
    let engine_version = info.version.trim().to_string();
    if engine_version.is_empty() {
        return Err(AppError::Other(format!(
            "TTS engine `{}` reported an empty version",
            engine.id()
        )));
    }
    Ok(BoundCache {
        cache,
        engine_id: engine.id().to_string(),
        engine_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::{EngineInfo, FakeTtsEngine, Speaker, SynthesizedAudio, TtsRequest};
    use async_trait::async_trait;

    #[test]
    fn key_changes_with_inputs() {
        let v = VoiceParams {
            speaker_id: 2,
            ..Default::default()
        };
        let a = TtsCache::key("voicevox", "0.22.0", "こんにちは", &v);
        let b = TtsCache::key("voicevox", "0.22.0", "こんにちは。", &v);
        let c = TtsCache::key(
            "voicevox",
            "0.22.0",
            "こんにちは",
            &VoiceParams {
                speed_scale: 1.1,
                ..v
            },
        );
        let d = TtsCache::key("fake", "0.22.0", "こんにちは", &v);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, TtsCache::key("voicevox", "0.22.0", "こんにちは", &v));
    }

    #[test]
    fn engine_version_is_part_of_the_key() {
        let v = VoiceParams::default();
        let old = TtsCache::key("voicevox", "0.21.1", "やあ", &v);
        let new = TtsCache::key("voicevox", "0.22.0", "やあ", &v);
        assert_ne!(old, new, "an engine upgrade must be a cache miss");
        assert_eq!(new, TtsCache::key("voicevox", "0.22.0", "やあ", &v));
    }

    #[test]
    fn entries_live_under_the_schema_directory() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TtsCache::new(dir.path());
        let key = TtsCache::key("x", "1", "y", &VoiceParams::default());
        let path = cache.put(&key, b"RIFFdata").unwrap();
        assert!(
            path.starts_with(dir.path().join("v2")),
            "{}",
            path.display()
        );
        assert_eq!(cache.schema_dir(), dir.path().join("v2"));

        // A schema-1 layout entry (no version directory) is never read back.
        let legacy = dir.path().join(&key[..2]).join(format!("{key}.wav"));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"RIFFlegacy").unwrap();
        assert_eq!(cache.get(&key).unwrap(), b"RIFFdata");
    }

    #[test]
    fn put_then_get() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TtsCache::new(dir.path());
        let key = TtsCache::key("x", "1", "y", &VoiceParams::default());
        assert!(cache.get(&key).is_none());
        cache.put(&key, b"RIFFdata").unwrap();
        assert_eq!(cache.get(&key).unwrap(), b"RIFFdata");
    }

    #[tokio::test]
    async fn bind_cache_uses_the_engine_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(TtsCache::new(dir.path()));
        let bound = bind_cache(&FakeTtsEngine::default(), cache).await.unwrap();
        assert_eq!(bound.engine_id, "fake");
        assert_eq!(bound.engine_version, "0");
        assert_eq!(
            bound.key("x", &VoiceParams::default()),
            TtsCache::key("fake", "0", "x", &VoiceParams::default())
        );
    }

    struct VersionlessEngine {
        version: &'static str,
    }

    #[async_trait]
    impl TtsEngine for VersionlessEngine {
        fn id(&self) -> &str {
            "versionless"
        }
        async fn health(&self) -> Result<EngineInfo, AppError> {
            if self.version == "unreachable" {
                return Err(AppError::VoicevoxUnavailable {
                    endpoint: "http://127.0.0.1:1".into(),
                    reason: "connection refused".into(),
                });
            }
            Ok(EngineInfo {
                engine: "versionless".into(),
                version: self.version.into(),
                endpoint: "in-process".into(),
            })
        }
        async fn list_speakers(&self) -> Result<Vec<Speaker>, AppError> {
            Ok(vec![])
        }
        async fn synthesize(&self, _: &TtsRequest) -> Result<SynthesizedAudio, AppError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn bind_cache_fails_when_the_version_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(TtsCache::new(dir.path()));

        let err = bind_cache(
            &VersionlessEngine {
                version: "unreachable",
            },
            Arc::clone(&cache),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "voicevox_unavailable");

        let err = bind_cache(&VersionlessEngine { version: "  " }, cache)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("empty version"), "{err}");
    }
}
