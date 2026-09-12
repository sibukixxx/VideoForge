//! Loader for a `png_lipsync` character model: three transparent PNGs
//! (closed / half-open / fully open mouth) that a renderer swaps between
//! based on a dialogue's lip-sync amplitude curve (P0-1,
//! `docs/character-system.md`). This is the PNG counterpart of
//! `live2d::load_model3_json` — offline, metadata/header-only, no pixel
//! decoding — used the same way by `videoforge-core::validate` and
//! `videoforge doctor`/`character inspect` before any render is attempted.

use std::path::Path;

use crate::png::{load_png_info, PngInfo};
use crate::{CharacterError, CharacterModel};

/// The three sprites a `png_lipsync` model resolves to, plus their header
/// info (used to warn about mismatched sprite sizes, which would make the
/// character visibly jump when the renderer swaps mouth states).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngLipsyncAssets {
    pub closed: PngInfo,
    pub half: PngInfo,
    pub open: PngInfo,
}

impl PngLipsyncAssets {
    /// `true` when the three sprites don't share the same pixel dimensions —
    /// not an error (a renderer can still scale each independently) but
    /// worth a warning: swapping mouth states will visibly shift the
    /// character.
    pub fn dimensions_mismatched(&self) -> bool {
        let dims = |i: &PngInfo| (i.width, i.height);
        dims(&self.closed) != dims(&self.half) || dims(&self.closed) != dims(&self.open)
    }
}

/// Load and validate a `png_lipsync` character's three sprites, resolved
/// against `manifest_dir` (same resolution rule as `CharacterModel::resolve_path`).
///
/// Callers must already know `model.model_type == MODEL_TYPE_PNG_LIPSYNC`;
/// `CharacterManifest::validate` guarantees `closed`/`half`/`open` are all
/// `Some` and non-empty for that model type, so a missing field here is an
/// internal invariant violation, not a user-facing error.
pub fn load_png_lipsync_assets(
    model: &CharacterModel,
    manifest_dir: &Path,
) -> Result<PngLipsyncAssets, CharacterError> {
    let closed = load_png_info(&model.resolve_closed(manifest_dir)?)?;
    let half = load_png_info(&model.resolve_half(manifest_dir)?)?;
    let open = load_png_info(&model.resolve_open(manifest_dir)?)?;
    Ok(PngLipsyncAssets { closed, half, open })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MODEL_TYPE_PNG_LIPSYNC;

    fn fixture_dir() -> std::path::PathBuf {
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/character/mock-png-character"
        ))
        .to_path_buf()
    }

    fn model(closed: &str, half: &str, open: &str) -> CharacterModel {
        CharacterModel {
            model_type: MODEL_TYPE_PNG_LIPSYNC.into(),
            path: None,
            closed: Some(closed.into()),
            half: Some(half.into()),
            open: Some(open.into()),
        }
    }

    #[test]
    fn loads_all_three_sprites() {
        let m = model(
            "./sprites/closed.png",
            "./sprites/half.png",
            "./sprites/open.png",
        );
        let assets = load_png_lipsync_assets(&m, &fixture_dir()).unwrap();
        assert_eq!((assets.closed.width, assets.closed.height), (8, 8));
        assert!(!assets.dimensions_mismatched());
    }

    #[test]
    fn rejects_missing_sprite() {
        let m = model(
            "./sprites/closed.png",
            "./sprites/does-not-exist.png",
            "./sprites/open.png",
        );
        let err = load_png_lipsync_assets(&m, &fixture_dir()).unwrap_err();
        assert!(matches!(err, CharacterError::PngNotFound { .. }));
    }

    #[test]
    fn rejects_opaque_sprite() {
        let m = model(
            "./sprites/no_alpha.png",
            "./sprites/half.png",
            "./sprites/open.png",
        );
        let err = load_png_lipsync_assets(&m, &fixture_dir()).unwrap_err();
        assert!(matches!(err, CharacterError::PngNotTransparent { .. }));
    }

    #[test]
    fn rejects_garbage_file() {
        let m = model(
            "./sprites/not_a_png.png",
            "./sprites/half.png",
            "./sprites/open.png",
        );
        let err = load_png_lipsync_assets(&m, &fixture_dir()).unwrap_err();
        assert!(matches!(err, CharacterError::PngInvalid { .. }));
    }
}
