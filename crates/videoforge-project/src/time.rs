/// Convert milliseconds to a frame index at the given fps (rounded to nearest).
///
/// Canonical time in the IR is milliseconds; frames are only computed at
/// export time so that changing fps never changes the project.
pub fn millis_to_frame(ms: u64, fps: u32) -> u32 {
    ((ms as f64 * fps as f64) / 1000.0).round() as u32
}

/// Inverse of [`millis_to_frame`], useful for tests and importers.
pub fn frame_to_millis(frame: u32, fps: u32) -> u64 {
    if fps == 0 {
        return 0;
    }
    ((frame as f64 * 1000.0) / fps as f64).round() as u64
}

/// Format milliseconds as `HH:MM:SS,mmm` (SRT style).
pub fn format_srt_timestamp(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// Format milliseconds as `HH:MM:SS.mmm` (dotted, used by YMM4 `TimeSpan`s).
pub fn format_timespan(ms: u64) -> String {
    format_srt_timestamp(ms).replace(',', ".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_conversion_rounds_to_nearest() {
        assert_eq!(millis_to_frame(0, 30), 0);
        assert_eq!(millis_to_frame(1000, 30), 30);
        assert_eq!(millis_to_frame(3410, 30), 102); // 102.3
        assert_eq!(millis_to_frame(3610, 30), 108); // 108.3
        assert_eq!(millis_to_frame(7290, 30), 219); // 218.7
        assert_eq!(millis_to_frame(1000, 60), 60);
        assert_eq!(millis_to_frame(16, 60), 1); // 0.96
    }

    #[test]
    fn frame_roundtrip() {
        assert_eq!(frame_to_millis(30, 30), 1000);
        assert_eq!(frame_to_millis(0, 0), 0);
    }

    #[test]
    fn srt_timestamp_format() {
        assert_eq!(format_srt_timestamp(0), "00:00:00,000");
        assert_eq!(format_srt_timestamp(3410), "00:00:03,410");
        assert_eq!(format_srt_timestamp(3_723_004), "01:02:03,004");
        assert_eq!(format_timespan(3410), "00:00:03.410");
    }
}
