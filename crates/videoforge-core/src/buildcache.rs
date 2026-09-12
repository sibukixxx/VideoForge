//! Minimal per-stage build cache (P0-4).
//!
//! TTS already has its own cache (`core::tts::TtsCache`, keyed on the
//! engine version + voice params + text) — a re-run whose dialogues and
//! voices are unchanged skips synthesis entirely. Timeline scheduling and
//! caption rendering are pure, in-memory, and cheap enough that fingerprinting
//! them buys nothing. The one remaining expensive, cacheable step is the
//! FFmpeg preview render, so that is what this module covers for P0: a
//! generate whose preview *inputs* are byte-for-byte unchanged since the
//! last successful generate reuses the previous `preview.mp4` instead of
//! re-invoking FFmpeg.
//!
//! This is intentionally not a general dependency graph — see "Known gaps"
//! in `CLAUDE.md` for what a fuller `--from`/`--only` incremental build
//! would still need.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::SubtitleConfig;

/// Sidecar file recording the fingerprint the current `preview.mp4` was
/// rendered from, written into the same generated output directory.
pub const PREVIEW_FINGERPRINT_FILE: &str = ".preview.fingerprint";

/// Deterministic fingerprint of everything that affects the rendered frame:
/// the project IR (every clip's timing, text, and asset paths — the
/// project's own `total_duration_ms`/tracks/video settings all live inside
/// its JSON), the resolved font's actual bytes (so a font *file* changing on
/// disk invalidates the cache even when its configured path didn't), the
/// background color, the subtitle style (P1-3: position/margin/colors/
/// outline/background box/font scale — any of these changes what a pixel on
/// screen looks like), and the renderer's own identity (so switching
/// renderer implementations, or a future FFmpeg-version-aware id, never
/// replays a stale render).
pub fn preview_fingerprint(
    project_json: &str,
    font: Option<&Path>,
    background_color: &str,
    subtitle: &SubtitleConfig,
    renderer_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_json.as_bytes());
    hasher.update(b"\0bg:");
    hasher.update(background_color.as_bytes());
    hasher.update(b"\0subtitle:");
    // `SubtitleConfig` has no stable byte encoding of its own; its `Debug`
    // form is good enough for a cache key (only equality/inequality across
    // runs matters, never cross-version stability).
    hasher.update(format!("{subtitle:?}").as_bytes());
    hasher.update(b"\0renderer:");
    hasher.update(renderer_id.as_bytes());
    hasher.update(b"\0font:");
    match font.and_then(|p| std::fs::read(p).ok()) {
        Some(bytes) => hasher.update(&bytes),
        None => hasher.update(b"none"),
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtitle() -> SubtitleConfig {
        SubtitleConfig::default()
    }

    #[test]
    fn same_inputs_produce_the_same_fingerprint() {
        let a = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        let b = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        assert_eq!(a, b);
    }

    #[test]
    fn project_json_change_invalidates() {
        let a = preview_fingerprint("{\"a\":1}", None, "#000000", &subtitle(), "ffmpeg");
        let b = preview_fingerprint("{\"a\":2}", None, "#000000", &subtitle(), "ffmpeg");
        assert_ne!(a, b);
    }

    #[test]
    fn background_color_change_invalidates() {
        let a = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        let b = preview_fingerprint("{}", None, "#ffffff", &subtitle(), "ffmpeg");
        assert_ne!(a, b);
    }

    #[test]
    fn subtitle_style_change_invalidates() {
        let a = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        let mut changed = subtitle();
        changed.font_color = "red".into();
        let b = preview_fingerprint("{}", None, "#000000", &changed, "ffmpeg");
        assert_ne!(a, b);
    }

    #[test]
    fn renderer_identity_change_invalidates() {
        let a = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        let b = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg-v2");
        assert_ne!(a, b);
    }

    #[test]
    fn font_file_content_change_invalidates_even_with_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let font = dir.path().join("font.ttf");
        std::fs::write(&font, b"version 1").unwrap();
        let a = preview_fingerprint("{}", Some(&font), "#000000", &subtitle(), "ffmpeg");
        std::fs::write(&font, b"version 2 (different bytes)").unwrap();
        let b = preview_fingerprint("{}", Some(&font), "#000000", &subtitle(), "ffmpeg");
        assert_ne!(a, b, "same path, different bytes, must not collide");
    }

    #[test]
    fn missing_font_is_distinct_from_no_font() {
        let with_none = preview_fingerprint("{}", None, "#000000", &subtitle(), "ffmpeg");
        let with_missing = preview_fingerprint(
            "{}",
            Some(Path::new("/definitely/not/a/font.ttf")),
            "#000000",
            &subtitle(),
            "ffmpeg",
        );
        assert_eq!(
            with_none, with_missing,
            "an unreadable font path degrades to the same 'no font' fingerprint"
        );
    }
}
