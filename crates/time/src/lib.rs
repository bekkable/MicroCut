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

    /// Whole milliseconds to ticks, saturating on overflow.
    pub fn from_millis(ms: i64) -> Tick {
        Tick(ms.saturating_mul(TICKS_PER_SECOND / 1000))
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

    /// Parses decimal seconds such as `"12.345000"` or `"-0.5"` exactly (no `f64`).
    ///
    /// Up to 6 fractional digits are exact; further digits are rounded to the nearest tick.
    /// Returns `None` for anything that is not a plain decimal number or does not fit.
    pub fn parse_decimal_secs(text: &str) -> Option<Tick> {
        const MAX_FRACTION_DIGITS: usize = 18;
        let text = text.trim();
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if (int_part.is_empty() && frac_part.is_empty()) || !all_digits(int_part) {
            return None;
        }
        if !all_digits(frac_part) {
            return None;
        }

        let whole: i128 = if int_part.is_empty() {
            0
        } else {
            int_part.parse().ok()?
        };
        let mut ticks = whole.checked_mul(i128::from(TICKS_PER_SECOND))?;

        let frac = frac_part.get(..frac_part.len().min(MAX_FRACTION_DIGITS))?;
        if !frac.is_empty() {
            let numerator: i128 = frac.parse().ok()?;
            let denominator = 10_i128.checked_pow(u32::try_from(frac.len()).ok()?)?;
            let scaled = numerator.checked_mul(i128::from(TICKS_PER_SECOND))?;
            // Round half up; the denominator is never zero (10^n, n >= 1).
            let rounded = scaled
                .checked_add(denominator / 2)?
                .checked_div(denominator)?;
            ticks = ticks.checked_add(rounded)?;
        }

        let ticks = if negative {
            ticks.checked_neg()?
        } else {
            ticks
        };
        i64::try_from(ticks).ok().map(Tick)
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

/// A non-negative time for display, formatted `hh:mm:ss.mmm` (milliseconds truncated).
///
/// Negative times format as `00:00:00.000`; hours grow beyond two digits if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockStamp(pub Tick);

impl fmt::Display for ClockStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ticks = self.0.0.max(0);
        let secs = ticks / TICKS_PER_SECOND;
        let millis = (ticks % TICKS_PER_SECOND) / (TICKS_PER_SECOND / 1000);
        let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        write!(f, "{h:02}:{m:02}:{s:02}.{millis:03}")
    }
}

/// A non-negative time for display, formatted `hh:mm:ss` (fraction truncated).
///
/// Negative times format as `00:00:00`; hours grow beyond two digits if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HmsStamp(pub Tick);

impl fmt::Display for HmsStamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.whole_secs().max(0);
        let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        write!(f, "{h:02}:{m:02}:{s:02}")
    }
}

/// A non-negative time as decimal seconds with microseconds (`12.345678`, truncated), the form
/// command-line tools such as ffmpeg expect. Negative times format as `0.000000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecimalSecs(pub Tick);

impl fmt::Display for DecimalSecs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ticks = self.0.0.max(0);
        let secs = ticks / TICKS_PER_SECOND;
        let micros = (ticks % TICKS_PER_SECOND) / (TICKS_PER_SECOND / 1_000_000);
        write!(f, "{secs}.{micros:06}")
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
    fn clock_stamp_shows_milliseconds() {
        let t = Tick::parse_decimal_secs("3725.0429").unwrap();
        assert_eq!(ClockStamp(t).to_string(), "01:02:05.042");
        assert_eq!(ClockStamp(Tick(-1)).to_string(), "00:00:00.000");
        assert_eq!(ClockStamp(Tick(i64::MAX)).to_string(), "10086:10:01.077");
    }

    #[test]
    fn hms_stamp_truncates_fraction() {
        let t = Tick::parse_decimal_secs("3725.999").unwrap();
        assert_eq!(HmsStamp(t).to_string(), "01:02:05");
        assert_eq!(HmsStamp(Tick(-1)).to_string(), "00:00:00");
    }

    #[test]
    fn decimal_secs_round_trips_through_parse() {
        for text in ["0.000000", "12.345678", "3600.000001"] {
            let tick = Tick::parse_decimal_secs(text).unwrap();
            assert_eq!(DecimalSecs(tick).to_string(), text);
        }
        assert_eq!(DecimalSecs(Tick(-5)).to_string(), "0.000000");
        // 1/60 s truncates to whole microseconds.
        let rate = FrameRate::new(60, 1).unwrap();
        assert_eq!(DecimalSecs(rate.frame_to_tick(1)).to_string(), "0.016666");
    }

    #[test]
    fn ntsc_frames_round_trip_exactly() {
        let rate = FrameRate::new(30_000, 1_001).unwrap();
        for frame in [0, 1, 29, 30, 1_798, 107_892] {
            assert_eq!(rate.tick_to_frame(rate.frame_to_tick(frame)), frame);
        }
    }

    #[test]
    fn parses_ffprobe_durations_exactly() {
        assert_eq!(Tick::parse_decimal_secs("12.345000"), {
            Some(Tick(
                12 * TICKS_PER_SECOND + 345 * (TICKS_PER_SECOND / 1000),
            ))
        });
        assert_eq!(
            Tick::parse_decimal_secs("0.000001"),
            Some(Tick(TICKS_PER_SECOND / 1_000_000))
        );
        assert_eq!(Tick::parse_decimal_secs("7"), Some(Tick::from_secs(7)));
        assert_eq!(
            Tick::parse_decimal_secs(".5"),
            Some(Tick(TICKS_PER_SECOND / 2))
        );
        assert_eq!(
            Tick::parse_decimal_secs("-1.5"),
            Some(Tick(-3 * TICKS_PER_SECOND / 2))
        );
        assert_eq!(Tick::parse_decimal_secs(" 3. "), Some(Tick::from_secs(3)));
    }

    #[test]
    fn rejects_hostile_decimal_text() {
        for bad in [
            "",
            ".",
            "-",
            "abc",
            "1.2.3",
            "1e5",
            "N/A",
            "١٢",
            "--1",
            "1.-5",
            "+-1",
            "99999999999999999999999999999999999999999999",
            "36400000000", // seconds that overflow i64 ticks
        ] {
            assert_eq!(Tick::parse_decimal_secs(bad), None, "{bad:?}");
        }
        // Very long fractions are truncated to 18 digits, not rejected.
        let long = format!("1.{}", "9".repeat(500));
        assert!(Tick::parse_decimal_secs(&long).is_some());
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
        assert_eq!(Tick::from_millis(i64::MAX), Tick(i64::MAX));
    }

    #[test]
    fn milliseconds() {
        assert_eq!(Tick::from_millis(1000), Tick::from_secs(1));
        assert_eq!(
            Tick::from_millis(-150),
            Tick::parse_decimal_secs("-0.15").unwrap()
        );
    }
}
