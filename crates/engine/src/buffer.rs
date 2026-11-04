//! Which frames to keep decoded around a paused cursor (the app's frame buffer), so stepping
//! frame by frame shows pictures at once.
//!
//! The window reaches `radius` frames to either side of the frame it was centred on. It stays
//! put while the cursor moves inside it with at least `radius / 2` frames left on either side;
//! otherwise it is centred on the cursor again, keeping the frames both windows share. The ends
//! of the clip cut it short, and a side that ends at the end of the clip never runs low.

/// The frames kept around the cursor: `first ..= last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferWindow {
    first: i64,
    last: i64,
    radius: i64,
}

impl BufferWindow {
    /// The window centred on `cursor`, `radius` frames to either side, within frames
    /// `0 ..= last_frame`.
    pub fn around(cursor: i64, radius: i64, last_frame: i64) -> BufferWindow {
        let radius = radius.max(0);
        let last_frame = last_frame.max(0);
        let cursor = cursor.clamp(0, last_frame);
        BufferWindow {
            first: cursor.saturating_sub(radius).max(0),
            last: cursor.saturating_add(radius).min(last_frame),
            radius,
        }
    }

    /// First frame of the window.
    pub fn first(&self) -> i64 {
        self.first
    }

    /// Last frame of the window (included).
    pub fn last(&self) -> i64 {
        self.last
    }

    /// Frames to either side of the centre.
    pub fn radius(&self) -> i64 {
        self.radius
    }

    /// Whether `frame` is in the window.
    pub fn contains(&self, frame: i64) -> bool {
        (self.first..=self.last).contains(&frame)
    }

    /// Whether the window still serves `cursor`: inside, with at least `radius / 2` frames on
    /// either side, unless that side ends at the clip's end (`0` or `last_frame`).
    pub fn serves(&self, cursor: i64, last_frame: i64) -> bool {
        if !self.contains(cursor) {
            return false;
        }
        let low = self.radius / 2;
        let before = self.first <= 0 || cursor.saturating_sub(self.first) >= low;
        let after = self.last >= last_frame || self.last.saturating_sub(cursor) >= low;
        before && after
    }

    /// The window for `cursor`: this one while it serves the cursor (and has `radius`), else a
    /// new one centred on the cursor.
    pub fn follow(self, cursor: i64, radius: i64, last_frame: i64) -> BufferWindow {
        if self.radius == radius.max(0) && self.serves(cursor, last_frame) {
            self
        } else {
            BufferWindow::around(cursor, radius, last_frame)
        }
    }

    /// The runs of consecutive frames in the window that `have` says are missing, as
    /// `(first frame, count)`, nearest to `cursor` first (a run containing the cursor comes
    /// first, then by distance).
    pub fn missing_runs(&self, cursor: i64, have: impl Fn(i64) -> bool) -> Vec<(i64, i64)> {
        let mut runs: Vec<(i64, i64)> = Vec::new();
        let mut start = None;
        for frame in self.first..=self.last {
            match (have(frame), start) {
                (false, None) => start = Some(frame),
                (true, Some(s)) => {
                    runs.push((s, frame - s));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, self.last - s + 1));
        }
        let distance = |&(first, count): &(i64, i64)| {
            let last = first + count - 1;
            if cursor < first {
                first - cursor
            } else if cursor > last {
                cursor - last
            } else {
                0
            }
        };
        runs.sort_by_key(distance);
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_reaches_radius_frames_each_way() {
        let w = BufferWindow::around(100, 32, 1000);
        assert_eq!((w.first(), w.last()), (68, 132));
        // Clip ends cut it short.
        let start = BufferWindow::around(5, 32, 1000);
        assert_eq!((start.first(), start.last()), (0, 37));
        let end = BufferWindow::around(995, 32, 1000);
        assert_eq!((end.first(), end.last()), (963, 1000));
        // Hostile numbers.
        let odd = BufferWindow::around(i64::MAX, i64::MAX, -5);
        assert_eq!((odd.first(), odd.last()), (0, 0));
    }

    #[test]
    fn it_stays_while_half_a_radius_is_left_each_way() {
        let w = BufferWindow::around(100, 32, 1000);
        // 16 frames each way are enough: 84 ..= 116.
        for cursor in 84..=116 {
            assert!(w.serves(cursor, 1000), "{cursor}");
            assert_eq!(w.follow(cursor, 32, 1000), w);
        }
        // More than 16 forward: fewer than 16 left ahead, so it moves to the cursor.
        assert!(!w.serves(117, 1000));
        let moved = w.follow(117, 32, 1000);
        assert_eq!((moved.first(), moved.last()), (85, 149));
        assert!(!w.serves(83, 1000));
        // A jump out of the window, too.
        assert_eq!(w.follow(500, 32, 1000).first(), 468);
        // A new radius (the config changed) rebuilds it.
        assert_eq!(w.follow(100, 8, 1000).last(), 108);
    }

    #[test]
    fn clip_ends_never_run_low() {
        let start = BufferWindow::around(0, 32, 1000);
        for cursor in 0..=16 {
            assert!(start.serves(cursor, 1000), "{cursor}");
        }
        let end = BufferWindow::around(1000, 32, 1000);
        assert!(end.serves(1000, 1000));
        assert!(end.serves(990, 1000));
        // A clip shorter than the window: always served.
        let short = BufferWindow::around(3, 32, 10);
        for cursor in 0..=10 {
            assert!(short.serves(cursor, 10));
        }
    }

    #[test]
    fn missing_runs_start_next_to_the_cursor() {
        let w = BufferWindow::around(100, 32, 1000); // 68 ..= 132
        // Nothing decoded: one run.
        assert_eq!(w.missing_runs(100, |_| false), [(68, 65)]);
        // All decoded: none.
        assert!(w.missing_runs(100, |_| true).is_empty());
        // Moved forward from a window 52 ..= 116: the new frames ahead come first.
        let have = |f: i64| (52..=116).contains(&f);
        assert_eq!(w.missing_runs(100, have), [(117, 16)]);
        // Gaps on both sides: the nearer one first.
        let have = |f: i64| (80..=125).contains(&f);
        assert_eq!(w.missing_runs(120, have), [(126, 7), (68, 12)]);
        // A run containing the cursor comes first.
        let have = |f: i64| !(95..=105).contains(&f) && f > 70;
        assert_eq!(w.missing_runs(100, have), [(95, 11), (68, 3)]);
    }
}
