//! Minimal PNG header reader (enough to validate a 3-state lip-sync sprite).
//!
//! Mirrors `videoforge-core`'s hand-rolled WAV header reader
//! (`videoforge_core::wav::parse_wav_info`): only the fixed-size `IHDR`
//! chunk is read, never the compressed pixel data, so a renderer built later
//! (or FFmpeg itself) is the only thing that ever decodes a frame. This is
//! enough to validate what P0-1 needs before spending render time: the file
//! is really a PNG, and it carries an alpha channel (a character sprite is
//! composited over a background, so a fully opaque PNG is very likely the
//! wrong export).

use std::path::Path;

use crate::CharacterError;

const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// PNG `IHDR` color types (see the PNG spec, §11.2.2). Only these five exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorType {
    Grayscale,
    Truecolor,
    Indexed,
    GrayscaleAlpha,
    TruecolorAlpha,
}

impl ColorType {
    fn from_byte(b: u8) -> Result<Self, String> {
        match b {
            0 => Ok(ColorType::Grayscale),
            2 => Ok(ColorType::Truecolor),
            3 => Ok(ColorType::Indexed),
            4 => Ok(ColorType::GrayscaleAlpha),
            6 => Ok(ColorType::TruecolorAlpha),
            other => Err(format!("unknown PNG color type {other}")),
        }
    }

    /// Whether this color type carries a full alpha channel. `Indexed`
    /// (palette) PNGs can carry partial transparency via a `tRNS` chunk, but
    /// that is deliberately not read here — a palette PNG is rejected with a
    /// message telling the author to export as RGBA instead of silently
    /// treating it as opaque or guessing at `tRNS`.
    pub fn has_alpha(self) -> bool {
        matches!(self, ColorType::GrayscaleAlpha | ColorType::TruecolorAlpha)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: ColorType,
}

impl PngInfo {
    pub fn has_alpha(&self) -> bool {
        self.color_type.has_alpha()
    }
}

/// Read just the `IHDR` chunk. Returns a plain `String` reason on failure —
/// same style as `videoforge_core::wav::parse_wav_info` — since callers wrap
/// it into their own error type with the path attached.
pub fn read_png_info(bytes: &[u8]) -> Result<PngInfo, String> {
    if bytes.len() < 8 + 8 + 13 || bytes[0..8] != SIGNATURE {
        return Err("not a PNG file (bad signature)".into());
    }
    let chunk_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if &bytes[12..16] != b"IHDR" {
        return Err("PNG's first chunk is not IHDR".into());
    }
    if chunk_len < 13 || bytes.len() < 16 + 13 {
        return Err("truncated IHDR chunk".into());
    }
    let ihdr = &bytes[16..16 + 13];
    let width = u32::from_be_bytes([ihdr[0], ihdr[1], ihdr[2], ihdr[3]]);
    let height = u32::from_be_bytes([ihdr[4], ihdr[5], ihdr[6], ihdr[7]]);
    if width == 0 || height == 0 {
        return Err("PNG has zero width or height".into());
    }
    let bit_depth = ihdr[8];
    let color_type = ColorType::from_byte(ihdr[9])?;
    Ok(PngInfo {
        width,
        height,
        bit_depth,
        color_type,
    })
}

/// Read and validate one PNG sprite file: must exist, be a well-formed PNG,
/// and carry an alpha channel (design requirement: "transparent PNG対応").
pub fn load_png_info(path: &Path) -> Result<PngInfo, CharacterError> {
    if !path.is_file() {
        return Err(CharacterError::PngNotFound {
            path: path.to_path_buf(),
        });
    }
    let bytes = std::fs::read(path).map_err(|e| CharacterError::PngInvalid {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let info = read_png_info(&bytes).map_err(|reason| CharacterError::PngInvalid {
        path: path.to_path_buf(),
        reason,
    })?;
    if !info.has_alpha() {
        return Err(CharacterError::PngNotTransparent {
            path: path.to_path_buf(),
        });
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal (structurally valid enough for header parsing) PNG:
    /// signature + IHDR + IEND, no pixel data. `read_png_info` never looks
    /// past IHDR so this is enough to exercise it without a real encoder.
    fn png_header_only(width: u32, height: u32, color_type: u8, bit_depth: u8) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.push(bit_depth);
        out.push(color_type);
        out.extend_from_slice(&[0, 0, 0]); // compression, filter, interlace
        out.extend_from_slice(&[0, 0, 0, 0]); // fake CRC, never checked
        out
    }

    #[test]
    fn reads_truecolor_alpha_header() {
        let bytes = png_header_only(64, 32, 6, 8);
        let info = read_png_info(&bytes).unwrap();
        assert_eq!(info.width, 64);
        assert_eq!(info.height, 32);
        assert_eq!(info.color_type, ColorType::TruecolorAlpha);
        assert!(info.has_alpha());
    }

    #[test]
    fn truecolor_without_alpha_is_reported_but_not_transparent() {
        let bytes = png_header_only(8, 8, 2, 8);
        let info = read_png_info(&bytes).unwrap();
        assert!(!info.has_alpha());
    }

    #[test]
    fn rejects_bad_signature() {
        assert!(read_png_info(b"not a png at all, just text").is_err());
    }

    #[test]
    fn rejects_non_ihdr_first_chunk() {
        let mut bytes = SIGNATURE.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IDAT");
        bytes.extend_from_slice(&[0u8; 13]);
        assert!(read_png_info(&bytes).is_err());
    }

    #[test]
    fn rejects_zero_dimensions() {
        let bytes = png_header_only(0, 8, 6, 8);
        assert!(read_png_info(&bytes).is_err());
    }

    #[test]
    fn load_png_info_rejects_missing_file() {
        let err = load_png_info(Path::new("/definitely/not/here.png")).unwrap_err();
        assert!(matches!(err, CharacterError::PngNotFound { .. }));
    }

    #[test]
    fn load_png_info_rejects_opaque_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opaque.png");
        std::fs::write(&path, png_header_only(8, 8, 2, 8)).unwrap();
        let err = load_png_info(&path).unwrap_err();
        assert!(matches!(err, CharacterError::PngNotTransparent { .. }));
    }

    #[test]
    fn load_png_info_accepts_real_fixture() {
        // Generated fixture with real (zlib-compressed) pixel data — proves
        // the header reader does not require the file to be header-only.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/character/mock-png-character/sprites/closed.png"
        );
        let info = load_png_info(Path::new(path)).unwrap();
        assert_eq!((info.width, info.height), (8, 8));
        assert!(info.has_alpha());
    }
}
