//! Exact media time for MicroCut (layer L0, no dependencies).
//!
//! All edit math uses [`Tick`], an integer count of 1/254 016 000 000 s. That rate is divisible by
//! every common video frame rate (23.976, 24, 25, 29.97, 30, 50, 59.94, 60, 120 …) and audio
//! sample rate, so frame positions are exact. Never use `f64` seconds for cut points.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::fmt;

/// Ticks per second.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point in (or span of) media time, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(pub i64);

impl Tick {
    /// Time zero.
    pub const ZERO: Tick = Tick(0);

    /// Whole seconds to ticks, saturating on overflow.
    pub fn from_secs(secs: i64) -> Tick {
        Tick(secs.saturating_mul(TICKS_PER_SECOND))
    }

    /// Whole seconds, rounded towards zero.
    pub fn whole_secs(self) -> i64 {
        self.0 / TICKS_PER_SECOND
    }

    /// Saturating addition.
    pub fn saturating_add(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_sub(rhs.0))
    }
}

/// A rational frame rate (`num / den` frames per second), e.g. 30000/1001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    num: u32,
    den: u32,
}

impl FrameRate {
    /// Builds a frame rate; `None` if numerator or denominator is zero.
    pub fn new(num: u32, den: u32) -> Option<FrameRate> {
        (num != 0 && den != 0).then_some(FrameRate { num, den })
    }

    /// Numerator.
    pub fn num(self) -> u32 {
        self.num
    }

    /// Denominator.
    pub fn den(self) -> u32 {
        self.den
    }

    /// Start time of frame `frame`, saturating on overflow.
    pub fn frame_to_tick(self, frame: i64) -> Tick {
        // The product can exceed i128 for absurd inputs, so saturate; the result is clamped anyway.
        let n = i128::from(frame)
            .saturating_mul(i128::from(TICKS_PER_SECOND))
            .saturating_mul(i128::from(self.den));
        Tick(clamp_i128(n.div_euclid(i128::from(self.num))))
    }

    /// Index of the frame that is showing at `tick` (floor).
    pub fn tick_to_frame(self, tick: Tick) -> i64 {
        let n = i128::from(tick.0).saturating_mul(i128::from(self.num));
        let d = i128::from(TICKS_PER_SECOND).saturating_mul(i128::from(self.den));
        clamp_i128(n.div_euclid(d))
    }
}

fn clamp_i128(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Hours, minutes and seconds of a non-negative time, formatted `hh-mm-ss` (file-name safe).
///
/// Negative times format as `00-00-00`; hours grow beyond two digits if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp(pub Tick);

impl fmt::Display for FileStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.whole_secs().max(0);
        let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        write!(f, "{h:02}-{m:02}-{s:02}")
    }
}

/// Range label for an exported cut, `hh-mm-ss-to-hh-mm-ss`; the export is named
/// `<stem>_<range label>.mp4`.
pub fn range_label(start: Tick, end: Tick) -> String {
    format!("{}-to-{}", FileStamp(start), FileStamp(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_label_formats_hours_minutes_seconds() {
        let start = Tick::from_secs(65);
        let end = Tick::from_secs(2 * 3600 + 3 * 60 + 4);
        assert_eq!(range_label(start, end), "00-01-05-to-02-03-04");
    }

    #[test]
    fn file_stamp_clamps_negative_and_truncates_fraction() {
        assert_eq!(FileStamp(Tick(-5)).to_string(), "00-00-00");
        let almost_two = Tick::from_secs(2).saturating_sub(Tick(1));
        assert_eq!(FileStamp(almost_two).to_string(), "00-00-01");
    }

    #[test]
    fn ntsc_frames_round_trip_exactly() {
        let rate = FrameRate::new(30_000, 1_001).unwrap();
        for frame in [0, 1, 29, 30, 1_798, 107_892] {
            assert_eq!(rate.tick_to_frame(rate.frame_to_tick(frame)), frame);
        }
    }

    #[test]
    fn zero_frame_rate_is_rejected() {
        assert!(FrameRate::new(0, 1).is_none());
        assert!(FrameRate::new(25, 0).is_none());
    }

    #[test]
    fn huge_values_saturate_instead_of_overflowing() {
        let rate = FrameRate::new(1, 1_000).unwrap();
        assert_eq!(rate.frame_to_tick(i64::MAX), Tick(i64::MAX));
        assert_eq!(Tick::from_secs(i64::MIN), Tick(i64::MIN));
    }
}
