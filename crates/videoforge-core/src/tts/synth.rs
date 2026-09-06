//! Batch synthesis with cache, concurrency limit, cancellation and progress.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::{BoundCache, TtsEngine, TtsRequest};
use crate::config::VoiceParams;
use crate::error::AppError;
use crate::progress::{GenerationStage, ProgressSink};
use crate::wav::parse_wav_info;

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesisJob {
    /// 1-based dialogue number.
    pub index: usize,
    pub speaker_key: String,
    pub speaker_display: String,
    pub text: String,
    pub voice: VoiceParams,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedDialogue {
    pub index: usize,
    pub speaker_key: String,
    pub speaker_display: String,
    pub text: String,
    pub wav: Vec<u8>,
    pub duration_ms: u64,
    pub cached: bool,
}

/// Synthesize all jobs. Results are returned in job order regardless of
/// completion order. Fails fast on the first error or cancellation.
///
/// `cache` must be bound to the engine build that will do the synthesis (see
/// [`super::bind_cache`]); pass `None` to bypass the cache entirely.
pub async fn synthesize_all(
    engine: Arc<dyn TtsEngine>,
    cache: Option<BoundCache>,
    jobs: Vec<SynthesisJob>,
    concurrency: usize,
    cancel: CancellationToken,
    progress: Arc<dyn ProgressSink>,
) -> Result<Vec<SynthesizedDialogue>, AppError> {
    let total = jobs.len();
    let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut set: JoinSet<Result<(usize, SynthesizedDialogue), AppError>> = JoinSet::new();
    let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    for (pos, job) in jobs.into_iter().enumerate() {
        let engine = Arc::clone(&engine);
        let cache = cache.clone();
        let semaphore = Arc::clone(&semaphore);
        let cancel = cancel.clone();
        let progress = Arc::clone(&progress);
        let done = Arc::clone(&done);
        set.spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|e| {
                AppError::InternalInvariant(format!("synthesis semaphore closed: {e}"))
            })?;
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let result = synthesize_one(engine.as_ref(), cache.as_ref(), &job, &cancel).await?;
            let current = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress.on_stage(&GenerationStage::Synthesizing {
                current,
                total,
                index: job.index,
                speaker: job.speaker_display.clone(),
                cached: result.cached,
            });
            Ok((pos, result))
        });
    }

    let mut results: Vec<Option<SynthesizedDialogue>> = (0..total).map(|_| None).collect();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok((pos, r))) => results[pos] = Some(r),
            Ok(Err(e)) => {
                set.abort_all();
                return Err(e);
            }
            Err(e) => {
                set.abort_all();
                return Err(AppError::InternalInvariant(format!(
                    "synthesis task failed to join: {e}"
                )));
            }
        }
    }
    results
        .into_iter()
        .enumerate()
        .map(|(pos, r)| {
            r.ok_or_else(|| {
                AppError::InternalInvariant(format!(
                    "synthesis job {pos} produced no result (expected {total})"
                ))
            })
        })
        .collect()
}

async fn synthesize_one(
    engine: &dyn TtsEngine,
    cache: Option<&BoundCache>,
    job: &SynthesisJob,
    cancel: &CancellationToken,
) -> Result<SynthesizedDialogue, AppError> {
    let key = cache.map(|c| c.key(&job.text, &job.voice));
    let hit = cache
        .zip(key.as_deref())
        .and_then(|(c, key)| c.cache.get(key));
    let (wav, cached) = match hit {
        Some(wav) if parse_wav_info(&wav).is_ok() => (wav, true),
        _ => {
            let request = TtsRequest {
                text: job.text.clone(),
                voice: job.voice,
            };
            let audio = tokio::select! {
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                r = engine.synthesize(&request) => r?,
            };
            if let Some((c, key)) = cache.zip(key.as_deref()) {
                if let Err(e) = c.cache.put(key, &audio.wav) {
                    tracing::warn!("tts cache write failed: {e}");
                }
            }
            (audio.wav, false)
        }
    };
    let info = parse_wav_info(&wav).map_err(|reason| AppError::VoicevoxSynthesisFailed {
        index: job.index,
        speaker: job.speaker_display.clone(),
        reason: format!("engine returned invalid WAV: {reason}"),
    })?;
    if info.duration_ms == 0 {
        return Err(AppError::VoicevoxSynthesisFailed {
            index: job.index,
            speaker: job.speaker_display.clone(),
            reason: "engine returned empty audio".into(),
        });
    }
    Ok(SynthesizedDialogue {
        index: job.index,
        speaker_key: job.speaker_key.clone(),
        speaker_display: job.speaker_display.clone(),
        text: job.text.clone(),
        wav,
        duration_ms: info.duration_ms,
        cached,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoopProgress;
    use crate::tts::{bind_cache, FakeTtsEngine, TtsCache};

    fn job(index: usize, text: &str) -> SynthesisJob {
        SynthesisJob {
            index,
            speaker_key: "reimu".into(),
            speaker_display: "霊夢".into(),
            text: text.into(),
            voice: VoiceParams::default(),
        }
    }

    #[tokio::test]
    async fn synthesizes_in_order_with_cache() {
        let dir = tempfile::tempdir().unwrap();
        let engine: Arc<dyn TtsEngine> = Arc::new(FakeTtsEngine::default());
        let cache = bind_cache(engine.as_ref(), Arc::new(TtsCache::new(dir.path())))
            .await
            .unwrap();
        let jobs = vec![job(1, "こんにちは"), job(2, "やあ"), job(3, "こんにちは")];

        let out = synthesize_all(
            Arc::clone(&engine),
            Some(cache.clone()),
            jobs.clone(),
            4,
            CancellationToken::new(),
            Arc::new(NoopProgress),
        )
        .await
        .unwrap();
        assert_eq!(
            out.iter().map(|d| d.index).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(out[0].duration_ms, 750);
        assert_eq!(out[1].duration_ms, 500);
        // job 3 has the same text as job 1: one of them hits the cache only if
        // it ran after the other finished; with concurrency both may miss.
        let again = synthesize_all(
            engine,
            Some(cache),
            jobs,
            1,
            CancellationToken::new(),
            Arc::new(NoopProgress),
        )
        .await
        .unwrap();
        assert!(again.iter().all(|d| d.cached));
    }

    #[tokio::test]
    async fn an_engine_upgrade_misses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let engine: Arc<dyn TtsEngine> = Arc::new(FakeTtsEngine::default());
        let store = Arc::new(TtsCache::new(dir.path()));
        let v1 = BoundCache {
            cache: Arc::clone(&store),
            engine_id: "fake".into(),
            engine_version: "0.21.1".into(),
        };
        let v2 = BoundCache {
            engine_version: "0.22.0".into(),
            ..v1.clone()
        };
        let run = |cache: BoundCache| {
            synthesize_all(
                Arc::clone(&engine),
                Some(cache),
                vec![job(1, "こんにちは")],
                1,
                CancellationToken::new(),
                Arc::new(NoopProgress),
            )
        };
        assert!(!run(v1.clone()).await.unwrap()[0].cached);
        assert!(run(v1).await.unwrap()[0].cached, "same version: hit");
        assert!(
            !run(v2.clone()).await.unwrap()[0].cached,
            "new version: miss"
        );
        assert!(run(v2).await.unwrap()[0].cached);
    }

    #[tokio::test]
    async fn cancelled_token_aborts() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = synthesize_all(
            Arc::new(FakeTtsEngine::default()),
            None,
            vec![job(1, "x")],
            1,
            cancel,
            Arc::new(NoopProgress),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
    }
}
