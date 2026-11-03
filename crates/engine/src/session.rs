//! The open video with its cursor, selected range and timeline view.

use std::sync::Arc;

use microcut_media::{MediaInfo, clamp_frame};
use microcut_time::{FrameRate, Tick};

use crate::view::View;

/// A non-empty run of frames, both ends included: `start..=end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRange {
    start: i64,
    end: i64,
}

impl FrameRange {
    /// First frame of the range.
    pub fn start(&self) -> i64 {
        self.start
    }

    /// Last frame of the range (included).
    pub fn end(&self) -> i64 {
        self.end
    }

    /// Number of frames in the range (at least 1).
    pub fn frame_count(&self) -> i64 {
        self.end.saturating_sub(self.start).saturating_add(1)
    }

    /// Time where the range starts (start of its first frame).
    pub fn start_time(&self, rate: FrameRate) -> Tick {
        rate.frame_to_tick(self.start)
    }

    /// Time where the range ends (end of its last frame).
    pub fn end_time(&self, rate: FrameRate) -> Tick {
        rate.frame_to_tick(self.end.saturating_add(1))
    }
}

/// One end of the selected range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeEnd {
    /// The first frame.
    Start,
    /// The last frame.
    End,
}

/// A side of the timeline view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Earlier frames.
    Left,
    /// Later frames.
    Right,
}

/// Edit state of one opened video.
#[derive(Debug, Clone)]
pub struct Session {
    media: Arc<MediaInfo>,
    /// Always within the clip.
    cursor: i64,
    /// Always within the clip, `start <= end`.
    range: FrameRange,
    view: View,
}

impl Session {
    /// A fresh session: cursor on the first frame, the whole clip selected, zoomed to fit.
    pub fn new(media: Arc<MediaInfo>) -> Session {
        let last = clamp_frame(&media, i64::MAX);
        let view = View::new(last.saturating_add(1));
        Session {
            media,
            cursor: 0,
            range: FrameRange {
                start: 0,
                end: last,
            },
            view,
        }
    }

    /// The opened video.
    pub fn media(&self) -> &Arc<MediaInfo> {
        &self.media
    }

    /// Frame rate of the video.
    pub fn frame_rate(&self) -> FrameRate {
        self.media.video.frame_rate
    }

    /// Index of the last frame.
    pub fn last_frame(&self) -> i64 {
        self.range_limit()
    }

    /// Frame at the cursor.
    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// Moves the cursor to `frame` (clamped to the clip).
    pub fn set_cursor(&mut self, frame: i64) {
        self.cursor = clamp_frame(&self.media, frame);
    }

    /// Moves the cursor to `frame` and scrolls the timeline so that it is visible.
    pub fn jump_cursor(&mut self, frame: i64) {
        self.set_cursor(frame);
        self.view.ensure_visible(self.cursor);
    }

    /// The selected range.
    pub fn range(&self) -> FrameRange {
        self.range
    }

    /// Moves the start of the range to `frame`, at most to its end. Returns the new start.
    pub fn set_range_start(&mut self, frame: i64) -> i64 {
        self.range.start = frame.clamp(0, self.range.end);
        self.range.start
    }

    /// Moves the end of the range to `frame` (included), at least to its start. Returns the new
    /// end.
    pub fn set_range_end(&mut self, frame: i64) -> i64 {
        self.range.end = frame.clamp(self.range.start, self.range_limit());
        self.range.end
    }

    /// Range ends that are scrolled out of view, per side: `(left, right)`. On each side it is
    /// the end nearest to the view — the one worth bringing to the cursor (both ends left of
    /// the view: the end; both right: the start).
    pub fn hidden_range_ends(&self) -> (Option<RangeEnd>, Option<RangeEnd>) {
        let view = &self.view;
        let start = view.x_of(self.range.start as f64);
        let end = view.x_of(self.range.end.saturating_add(1) as f64);
        // Half a pixel of slack: an end exactly at the edge is visible.
        let (min, max) = (-0.5, view.width() + 0.5);
        let left = if end < min {
            Some(RangeEnd::End)
        } else if start < min {
            Some(RangeEnd::Start)
        } else {
            None
        };
        let right = if start > max {
            Some(RangeEnd::Start)
        } else if end > max {
            Some(RangeEnd::End)
        } else {
            None
        };
        (left, right)
    }

    /// Moves the range end hidden on `side` (the one [`Session::hidden_range_ends`] names) to
    /// the first (left) or last (right) fully visible frame. Returns which end moved, if any.
    pub fn bring_hidden_end(&mut self, side: Side) -> Option<RangeEnd> {
        let (left, right) = self.hidden_range_ends();
        let (which, pos) = match side {
            Side::Left => (left?, self.view.left().ceil()),
            Side::Right => (right?, self.view.right().floor() - 1.0),
        };
        let frame = pos.clamp(0.0, self.range_limit() as f64) as i64;
        self.set_range_end_to(which, frame);
        Some(which)
    }

    /// Moves `which` end of the range to `frame` (clamped as by [`Session::set_range_start`] /
    /// [`Session::set_range_end`]). Returns where it ended up.
    pub fn set_range_end_to(&mut self, which: RangeEnd, frame: i64) -> i64 {
        match which {
            RangeEnd::Start => self.set_range_start(frame),
            RangeEnd::End => self.set_range_end(frame),
        }
    }

    /// The timeline view.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// The timeline view, for zooming and scrolling.
    pub fn view_mut(&mut self) -> &mut View {
        &mut self.view
    }

    fn range_limit(&self) -> i64 {
        clamp_frame(&self.media, i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use microcut_media::VideoInfo;

    use super::*;

    fn session(secs: i64, fps: u32) -> Session {
        let media = MediaInfo {
            path: PathBuf::from("clip.mp4"),
            container: "mp4".to_owned(),
            start: Tick::ZERO,
            duration: Tick::from_secs(secs),
            video: VideoInfo {
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(fps, 1).unwrap(),
                rotation: 0,
            },
            audio: None,
        };
        Session::new(Arc::new(media))
    }

    #[test]
    fn new_session_selects_everything() {
        let s = session(10, 25);
        assert_eq!(s.cursor(), 0);
        assert_eq!(s.last_frame(), 249);
        assert_eq!(s.range().start(), 0);
        assert_eq!(s.range().end(), 249);
        assert_eq!(s.range().frame_count(), 250);
        assert_eq!(s.range().end_time(s.frame_rate()), Tick::from_secs(10));
    }

    #[test]
    fn cursor_stays_in_the_clip() {
        let mut s = session(10, 25);
        s.set_cursor(-3);
        assert_eq!(s.cursor(), 0);
        s.set_cursor(i64::MAX);
        assert_eq!(s.cursor(), 249);
    }

    #[test]
    fn range_ends_cannot_cross() {
        let mut s = session(10, 25);
        assert_eq!(s.set_range_end(100), 100);
        assert_eq!(s.set_range_start(150), 100, "start stops at the end");
        assert_eq!(s.set_range_start(-5), 0);
        assert_eq!(s.set_range_start(40), 40);
        assert_eq!(s.set_range_end(10), 40, "end stops at the start");
        assert_eq!(s.set_range_end(i64::MAX), 249);
        assert_eq!(s.range().frame_count(), 210);
        assert_eq!(
            s.range().start_time(s.frame_rate()),
            Tick(40 * 254_016_000_000 / 25)
        );
    }

    #[test]
    fn hidden_range_ends_point_to_the_nearest_end() {
        let mut s = session(100, 25); // 2500 frames
        s.view_mut().set_width(1000.0);
        assert_eq!(s.hidden_range_ends(), (None, None), "whole clip visible");

        s.view_mut().zoom_at(0.0, 0.4); // 2.5 → 1 frame per px
        s.view_mut().set_left(1000.0); // frames 1000..2000 visible
        // Range 0..=2499 covers the view: start left, end right.
        assert_eq!(
            s.hidden_range_ends(),
            (Some(RangeEnd::Start), Some(RangeEnd::End))
        );
        // Range 100..=500 entirely left of the view: bring its end.
        s.set_range_start(100);
        s.set_range_end(500);
        assert_eq!(s.hidden_range_ends(), (Some(RangeEnd::End), None));
        // Range 2100..=2400 entirely right: bring its start.
        s.set_range_end(2400);
        s.set_range_start(2100);
        assert_eq!(s.hidden_range_ends(), (None, Some(RangeEnd::Start)));
        // Range 1200..=1500 visible.
        s.set_range_start(1200);
        s.set_range_end(1500);
        assert_eq!(s.hidden_range_ends(), (None, None));
    }

    #[test]
    fn hidden_ends_come_to_the_edge_of_the_view() {
        let mut s = session(100, 25); // 2500 frames
        s.view_mut().set_width(1000.0);
        s.view_mut().zoom_at(0.0, 0.4); // 1 frame per px
        s.view_mut().set_left(1000.5); // frames 1000.5..2000.5 visible
        let range = |s: &Session| (s.range().start(), s.range().end());

        // Whole clip selected: the start comes to the first, the end to the last full frame.
        assert_eq!(s.bring_hidden_end(Side::Left), Some(RangeEnd::Start));
        assert_eq!(range(&s), (1001, 2499));
        assert_eq!(s.bring_hidden_end(Side::Right), Some(RangeEnd::End));
        assert_eq!(range(&s), (1001, 1999));
        assert_eq!(s.hidden_range_ends(), (None, None));
        assert_eq!(s.bring_hidden_end(Side::Left), None, "nothing hidden");

        // Range entirely left of the view: its end comes to the left edge (range grows).
        s.set_range_start(100);
        s.set_range_end(500);
        assert_eq!(s.bring_hidden_end(Side::Left), Some(RangeEnd::End));
        assert_eq!(range(&s), (100, 1001));

        // Range entirely right of the view: its start comes to the right edge.
        s.set_range_end(2400);
        s.set_range_start(2100);
        assert_eq!(s.bring_hidden_end(Side::Right), Some(RangeEnd::Start));
        assert_eq!(range(&s), (1999, 2400));
    }

    #[test]
    fn jump_scrolls_the_cursor_into_view() {
        let mut s = session(100, 25); // 2500 frames
        s.view_mut().set_width(100.0);
        s.view_mut().zoom_at(0.0, 0.01); // 0.25 frames per px: frames 0..25 visible
        s.jump_cursor(2000);
        let x = s.view().x_of(2000.0);
        assert!((0.0..100.0).contains(&x));
    }
}
