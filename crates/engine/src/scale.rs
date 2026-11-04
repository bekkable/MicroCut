//! Marks for the time scale and the frame scale, spaced to suit the zoom.
//!
//! A scale picks the smallest "nice" step whose labels are at least `min_label_px` apart and
//! subdivides it with unlabelled minor marks when there is room.

use microcut_time::{ClockStamp, FrameRate, HmsStamp, TICKS_PER_SECOND, Tick};

use crate::view::View;

/// Minor marks closer than this are left out.
const MIN_MINOR_PX: f64 = 6.0;

/// Never produce more marks than this, whatever the numbers say.
const MAX_MARKS: usize = 2_000;

/// Time steps in milliseconds with the number of minor divisions per step.
const TIME_STEPS: &[(i64, i64)] = &[
    (10, 5),
    (20, 4),
    (50, 5),
    (100, 5),
    (200, 4),
    (500, 5),
    (1_000, 5),
    (2_000, 4),
    (5_000, 5),
    (10_000, 5),
    (15_000, 3),
    (30_000, 3),
    (60_000, 4),
    (120_000, 4),
    (300_000, 5),
    (600_000, 5),
    (900_000, 3),
    (1_800_000, 3),
    (3_600_000, 4),
    (7_200_000, 4),
    (21_600_000, 6),
    (43_200_000, 4),
    (86_400_000, 4),
];

/// One mark on a scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// Position from the left edge of the timeline, in the view's pixels.
    pub x: f64,
    /// Text for a major mark; `None` for a minor mark.
    pub label: Option<String>,
}

/// Marks for the time scale: times since the start of the clip, labelled `hh:mm:ss`
/// (`hh:mm:ss.mmm` below one second).
pub fn time_marks(view: &View, rate: FrameRate, min_label_px: f64) -> Vec<Mark> {
    let Some(TimeUnit {
        step_ms,
        unit_ms,
        every,
    }) = time_unit(view, rate, min_label_px)
    else {
        return Vec::new();
    };
    let fps = f64::from(rate.num()) / f64::from(rate.den());

    let ms_at = |pos: f64| pos / fps * 1000.0;
    let first = (ms_at(view.left()) / unit_ms as f64).floor().max(0.0) as i64;
    let last = (ms_at(view.right().min(view.end())) / unit_ms as f64).ceil() as i64;
    let ticks_per_ms = TICKS_PER_SECOND / 1000;
    let tick_to_pos = |tick: i64| tick as f64 / TICKS_PER_SECOND as f64 * fps;

    (first..=last)
        .take(MAX_MARKS)
        .filter_map(|n| {
            let tick = n.checked_mul(unit_ms)?.checked_mul(ticks_per_ms)?;
            let pos = tick_to_pos(tick);
            if pos > view.end() + 1e-9 {
                return None;
            }
            let label = (n % every == 0).then(|| {
                if step_ms < 1_000 {
                    ClockStamp(Tick(tick)).to_string()
                } else {
                    HmsStamp(Tick(tick)).to_string()
                }
            });
            Some(Mark {
                x: view.x_of(pos),
                label,
            })
        })
        .collect()
}

/// Spacing of the time scale at a zoom level.
struct TimeUnit {
    /// Between labelled marks.
    step_ms: i64,
    /// Between drawn marks (labelled or not).
    unit_ms: i64,
    /// Drawn marks per labelled one.
    every: i64,
}

fn time_unit(view: &View, rate: FrameRate, min_label_px: f64) -> Option<TimeUnit> {
    let fps = f64::from(rate.num()) / f64::from(rate.den());
    let px_per_ms = fps / view.frames_per_px() / 1000.0;
    if !px_per_ms.is_finite() || px_per_ms <= 0.0 {
        return None;
    }
    let (step_ms, divisions) = TIME_STEPS
        .iter()
        .copied()
        .find(|&(ms, _)| ms as f64 * px_per_ms >= min_label_px)
        .unwrap_or((86_400_000, 4));
    let (unit_ms, every) = subdivide(step_ms, divisions, px_per_ms);
    Some(TimeUnit {
        step_ms,
        unit_ms,
        every,
    })
}

/// The frame boundary nearest to boundary position `pos`, snapped to the marks of the time
/// scale at this zoom (Shift while dragging a range handle). Unsnapped when the marks are
/// closer than a frame.
pub fn snap_boundary(view: &View, rate: FrameRate, min_label_px: f64, pos: f64) -> i64 {
    let unsnapped = pos.round().clamp(0.0, view.end()) as i64;
    let Some(TimeUnit { unit_ms, .. }) = time_unit(view, rate, min_label_px) else {
        return unsnapped;
    };
    let (num, den) = (i128::from(rate.num()), i128::from(rate.den()));
    let unit_ms = i128::from(unit_ms);
    // Marks closer than one frame (unit_ms / 1000 < den / num): frames are the finer grid.
    if unit_ms * num < 1000 * den {
        return unsnapped;
    }
    snap_to_unit(view, rate, unit_ms, pos).unwrap_or(unsnapped)
}

/// The frame boundary nearest to boundary position `pos`, snapped to whole seconds since the
/// start of the clip, whatever the zoom (Ctrl while dragging a range handle or the cursor).
pub fn snap_boundary_to_seconds(view: &View, rate: FrameRate, pos: f64) -> i64 {
    let unsnapped = pos.round().clamp(0.0, view.end()) as i64;
    snap_to_unit(view, rate, 1000, pos).unwrap_or(unsnapped)
}

/// The frame boundary nearest to the multiple of `unit_ms` nearest to boundary position `pos`,
/// within `0 ..= view.end()`; `None` for positions that are not numbers.
fn snap_to_unit(view: &View, rate: FrameRate, unit_ms: i128, pos: f64) -> Option<i64> {
    let (num, den) = (i128::from(rate.num()), i128::from(rate.den()));
    let fps = f64::from(rate.num()) / f64::from(rate.den());
    let ms = pos / fps * 1000.0;
    let marks = (ms / unit_ms.max(1) as f64).round();
    if !marks.is_finite() {
        return None;
    }
    // Mark time in ms → nearest frame boundary, exactly: frames = ms · num / (1000 · den).
    let mark_ms = (marks as i128).saturating_mul(unit_ms);
    let scaled = mark_ms.saturating_mul(num);
    let divisor = 1000 * den;
    let frame = (scaled + divisor / 2).div_euclid(divisor);
    let end = view.end() as i128;
    i64::try_from(frame.clamp(0, end)).ok()
}

/// Marks for the frame scale: frame numbers at the start of each labelled frame.
pub fn frame_marks(view: &View, min_label_px: f64) -> Vec<Mark> {
    let px_per_frame = 1.0 / view.frames_per_px();
    if !px_per_frame.is_finite() || px_per_frame <= 0.0 {
        return Vec::new();
    }
    let (step, divisions) = frame_step(px_per_frame, min_label_px);
    let (unit, every) = subdivide(step, divisions, px_per_frame);

    let first = (view.left() / unit as f64).floor().max(0.0) as i64;
    let last = (view.right().min(view.end() - 1.0) / unit as f64).ceil() as i64;
    (first..=last)
        .take(MAX_MARKS)
        .filter_map(|n| {
            let frame = n.checked_mul(unit)?;
            let pos = frame as f64;
            if pos >= view.end() {
                return None;
            }
            Some(Mark {
                x: view.x_of(pos),
                label: (n % every == 0).then(|| frame.to_string()),
            })
        })
        .collect()
}

/// Smallest step of 1, 2, 5, 10, 20, 50, … frames whose labels fit, with its minor divisions.
fn frame_step(px_per_frame: f64, min_label_px: f64) -> (i64, i64) {
    let mut magnitude: i64 = 1;
    loop {
        for (factor, divisions) in [(1, 10), (2, 2), (5, 5)] {
            let step = magnitude.saturating_mul(factor);
            if step as f64 * px_per_frame >= min_label_px {
                // A step of one frame cannot be subdivided.
                return (step, if step == 1 { 1 } else { divisions });
            }
        }
        match magnitude.checked_mul(10) {
            Some(next) => magnitude = next,
            None => return (magnitude, 1),
        }
    }
}

/// The spacing of drawn marks (`unit`) and how many of them make one labelled step (`every`):
/// minor marks only if they are far enough apart.
fn subdivide(step: i64, divisions: i64, px_per_unit: f64) -> (i64, i64) {
    let minor = step / divisions.max(1);
    if minor >= 1 && step % divisions.max(1) == 0 && minor as f64 * px_per_unit >= MIN_MINOR_PX {
        (minor, divisions.max(1))
    } else {
        (step, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(frames: i64, width: f64) -> View {
        let mut view = View::new(frames);
        view.set_width(width);
        view
    }

    fn labels(marks: &[Mark]) -> Vec<&str> {
        marks.iter().filter_map(|m| m.label.as_deref()).collect()
    }

    #[test]
    fn whole_minute_clip_gets_ten_second_labels() {
        // 60 s at 25 fps in 600 px: 10 px per second, labels at least 80 px apart.
        let v = view(1500, 600.0);
        let rate = FrameRate::new(25, 1).unwrap();
        let marks = time_marks(&v, rate, 80.0);
        assert_eq!(
            labels(&marks),
            [
                "00:00:00", "00:00:10", "00:00:20", "00:00:30", "00:00:40", "00:00:50", "00:01:00"
            ]
        );
        // Minor marks every 2 s (20 px).
        assert_eq!(marks.len(), 31);
        assert!((marks[1].x - 20.0).abs() < 1e-9);
        assert!((marks[30].x - 600.0).abs() < 1e-9);
    }

    #[test]
    fn deep_zoom_labels_milliseconds() {
        let mut v = view(6000, 1000.0);
        let rate = FrameRate::new(60, 1).unwrap();
        v.zoom_at(0.0, 1e-9); // 120 px per frame = 7200 px per second
        let marks = time_marks(&v, rate, 80.0);
        assert_eq!(
            labels(&marks)[..3],
            ["00:00:00.000", "00:00:00.020", "00:00:00.040"]
        );
    }

    #[test]
    fn ntsc_rate_positions_are_exact_enough() {
        let rate = FrameRate::new(30_000, 1_001).unwrap();
        // One hour of NTSC video in 3600 px.
        let v = view(107_892, 3600.0);
        // About 1 px per second: labels every 2 min, minor marks every 30 s.
        let marks = time_marks(&v, rate, 80.0);
        let last = marks.iter().rev().find(|m| m.label.is_some()).unwrap();
        assert_eq!(last.label.as_deref(), Some("00:58:00"));
        // 58 min = 3480 s = 104 295.7 frames → px = frames / frames_per_px.
        let expected = 3480.0 * 30_000.0 / 1_001.0 / v.frames_per_px();
        assert!((last.x - expected).abs() < 1e-6);
    }

    #[test]
    fn snapping_follows_the_time_marks() {
        let rate = FrameRate::new(60, 1).unwrap();
        // 60 s at 60 fps in 600 px: labels every 10 s, marks every 2 s.
        let v = view(3600, 600.0);
        assert_eq!(snap_boundary(&v, rate, 80.0, 125.0), 120);
        // 185 frames = 3.08 s → nearest mark 4 s.
        assert_eq!(snap_boundary(&v, rate, 80.0, 185.0), 240);
        assert_eq!(snap_boundary(&v, rate, 80.0, 1e12), 3600);
        assert_eq!(snap_boundary(&v, rate, 80.0, -50.0), 0);
        assert_eq!(snap_boundary(&v, rate, 80.0, f64::NAN), 0);
    }

    #[test]
    fn snapping_finds_the_nearest_frame_for_ntsc() {
        let rate = FrameRate::new(30_000, 1_001).unwrap();
        // 1 px per frame: labels every 5 s, marks every 1 s (29.97 px).
        let v = view(10_000, 10_000.0);
        // 1 s = 29.97 frames → boundary 30; 2 s = 59.94 → 60; 3 s = 89.91 → 90.
        assert_eq!(snap_boundary(&v, rate, 80.0, 31.0), 30);
        assert_eq!(snap_boundary(&v, rate, 80.0, 88.0), 90);
        // 100 s = 2997.0 frames exactly.
        assert_eq!(snap_boundary(&v, rate, 80.0, 2990.0), 2997);
    }

    #[test]
    fn ctrl_snaps_to_whole_seconds_at_any_zoom() {
        let rate = FrameRate::new(60, 1).unwrap();
        // Zoomed out (scale marks every 2 s): Shift → 4 s, whole seconds → 3 s.
        let v = view(3600, 600.0);
        assert_eq!(snap_boundary(&v, rate, 80.0, 185.0), 240);
        assert_eq!(snap_boundary_to_seconds(&v, rate, 185.0), 180);
        // Zoomed in (marks every few ms): still whole seconds.
        let mut deep = view(6000, 1000.0);
        deep.zoom_at(0.0, 1e-9);
        assert_eq!(snap_boundary_to_seconds(&deep, rate, 89.0), 60);
        assert_eq!(snap_boundary_to_seconds(&deep, rate, 91.0), 120);
        // NTSC: 1 s = 29.97 frames → boundary 30; 100 s = 2997 exactly.
        let ntsc = FrameRate::new(30_000, 1_001).unwrap();
        let v = view(10_000, 10_000.0);
        assert_eq!(snap_boundary_to_seconds(&v, ntsc, 31.0), 30);
        assert_eq!(snap_boundary_to_seconds(&v, ntsc, 2990.0), 2997);
        // Clamped to the clip; nonsense stays inside.
        assert_eq!(snap_boundary_to_seconds(&v, ntsc, 1e12), 10_000);
        assert_eq!(snap_boundary_to_seconds(&v, ntsc, -50.0), 0);
        assert_eq!(snap_boundary_to_seconds(&v, ntsc, f64::NAN), 0);
    }

    #[test]
    fn deep_zoom_snaps_to_frames() {
        let mut v = view(6000, 1000.0);
        let rate = FrameRate::new(60, 1).unwrap();
        v.zoom_at(0.0, 1e-9); // marks every 5 ms, a frame is 16.7 ms
        assert_eq!(snap_boundary(&v, rate, 80.0, 3.4), 3);
        assert_eq!(snap_boundary(&v, rate, 80.0, 3.6), 4);
    }

    #[test]
    fn frame_marks_label_round_numbers() {
        // 1000 frames in 1000 px: labels every 100 frames, minor every 10.
        let v = view(1000, 1000.0);
        let marks = frame_marks(&v, 80.0);
        assert_eq!(labels(&marks)[..3], ["0", "100", "200"]);
        assert_eq!(labels(&marks).last(), Some(&"900"));
        assert!((marks[1].x - 10.0).abs() < 1e-9);
        assert_eq!(marks.len(), 100);
    }

    #[test]
    fn frame_step_sequence() {
        assert_eq!(frame_step(100.0, 50.0), (1, 1));
        assert_eq!(frame_step(30.0, 50.0), (2, 2));
        assert_eq!(frame_step(1.0, 50.0), (50, 5));
        assert_eq!(frame_step(0.6, 50.0), (100, 10));
        assert_eq!(frame_step(1e-30, 50.0), (1_000_000_000_000_000_000, 1));
    }

    #[test]
    fn marks_stay_bounded_and_finite() {
        let v = view(i64::MAX, 1000.0);
        let rate = FrameRate::new(1000, 1).unwrap();
        assert!(time_marks(&v, rate, 80.0).len() <= MAX_MARKS);
        assert!(frame_marks(&v, 50.0).len() <= MAX_MARKS);
        for mark in time_marks(&v, rate, 80.0) {
            assert!(mark.x.is_finite());
        }
    }
}
