//! Render presets (P1-6): named bundles of resolution/fps/codec/quality/
//! audio settings, so a user picks `youtube-1080p` instead of setting five
//! separate knobs by hand. A preset overrides `VideoProject::video`
//! (width/height/fps — safe because every clip's `Transform` position is a
//! normalized `0.0..=1.0` fraction, not a pixel value, so changing the
//! render resolution never touches placement math) and supplies the
//! renderer's [`crate::preview::EncodeSettings`]. `videoforge generate
//! --preset <name>` and the fast-preview command (P1-7) both take a preset
//! by name; this module is the single source of truth for what each name
//! means.

use crate::preview::EncodeSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderPreset {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub encode: EncodeSettings,
}

pub const PRESETS: &[RenderPreset] = &[
    // A conventional 1080p landscape upload: high quality, moderate encode
    // speed — this is the deliverable, not a quick check.
    RenderPreset {
        name: "youtube-1080p",
        width: 1920,
        height: 1080,
        fps: 30,
        encode: EncodeSettings {
            video_codec: "libx264",
            encoder_speed: "medium",
            crf: 18,
            audio_bitrate_kbps: 192,
        },
    },
    // A vertical short-form upload (9:16). Same quality target as
    // youtube-1080p; only the aspect ratio and a slightly higher CRF differ.
    RenderPreset {
        name: "youtube-short",
        width: 1080,
        height: 1920,
        fps: 30,
        encode: EncodeSettings {
            video_codec: "libx264",
            encoder_speed: "medium",
            crf: 20,
            audio_bitrate_kbps: 192,
        },
    },
    // Small and fast: for checking timing/composition, not for judging
    // final image quality. Pairs naturally with P1-7's range-limited render.
    RenderPreset {
        name: "preview-low",
        width: 960,
        height: 540,
        fps: 24,
        encode: EncodeSettings {
            video_codec: "libx264",
            encoder_speed: "ultrafast",
            crf: 30,
            audio_bitrate_kbps: 128,
        },
    },
];

/// Look up a preset by its exact name (case-sensitive — these are fixed
/// identifiers, not free text).
pub fn find(name: &str) -> Option<&'static RenderPreset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// Every known preset name, for CLI help text and error messages.
pub fn names() -> Vec<&'static str> {
    PRESETS.iter().map(|p| p.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_known_preset_by_name() {
        for name in names() {
            assert_eq!(find(name).unwrap().name, name);
        }
    }

    #[test]
    fn unknown_name_is_none() {
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn every_preset_differs_from_the_default_encode_settings() {
        // Otherwise a preset render and a default render could produce a
        // fingerprint collision that looks like a correct cache hit despite
        // different resolutions - width/height/fps are hashed via the
        // project JSON regardless, so this isn't strictly required for
        // correctness, but distinct settings are the intent of "preset".
        for preset in PRESETS {
            assert_ne!(preset.encode, EncodeSettings::default());
        }
    }

    #[test]
    fn names_lists_every_preset_exactly_once() {
        let names = names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
        assert_eq!(names.len(), PRESETS.len());
    }
}
