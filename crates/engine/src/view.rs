//! The visible part of the timeline: which frames, at which zoom.
//!
//! Positions are in frame units: frame `i` covers `[i, i + 1)`, the clip covers `[0, frames]`.
//! `x` is in pixels (or points) from the left edge of the timeline.

/// Deepest zoom: pixels per frame.
pub const MAX_PX_PER_FRAME: f64 = 120.0;

/// Zoom and scroll position of the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Frame count of the clip (at least 1).
    frames: f64,
    /// Width of the timeline (at least 1).
    width: f64,
    /// Frame position at `x = 0`.
    left: f64,
    /// Frames per pixel.
    per_px: f64,
}

impl View {
    /// A view of a clip with `frame_count` frames, zoomed to fit a width of one pixel; call
    /// [`View::set_width`] before use.
    pub fn new(frame_count: i64) -> View {
        let frames = frame_count.max(1) as f64;
        View {
            frames,
            width: 1.0,
            left: 0.0,
            per_px: frames,
        }
    }

    /// Width of the timeline.
    pub fn width(&self) -> f64 {
        self.width
    }

    /// Frames per pixel.
    pub fn frames_per_px(&self) -> f64 {
        self.per_px
    }

    /// Frame position at the left edge.
    pub fn left(&self) -> f64 {
        self.left
    }

    /// Frame position at the right edge (may lie beyond the clip if the clip is short).
    pub fn right(&self) -> f64 {
        self.pos_at(self.width)
    }

    /// Frame count of the clip as a position (end of the last frame).
    pub fn end(&self) -> f64 {
        self.frames
    }

    /// Sets the timeline width. A view that showed the whole clip keeps showing it.
    pub fn set_width(&mut self, width: f64) {
        let width = if width.is_finite() {
            width.max(1.0)
        } else {
            1.0
        };
        if width == self.width {
            return;
        }
        let fitted = self.is_fitted();
        self.width = width;
        if fitted {
            self.fit();
        } else {
            self.per_px = self.per_px.clamp(min_per_px(), self.max_per_px());
            self.clamp_left();
        }
    }

    /// Whether the whole clip is visible (zoomed out completely).
    pub fn is_fitted(&self) -> bool {
        self.per_px >= self.max_per_px() * (1.0 - 1e-9)
    }

    /// Zooms out to show the whole clip.
    pub fn fit(&mut self) {
        self.per_px = self.max_per_px();
        self.left = 0.0;
    }

    /// Multiplies frames per pixel by `factor` (`< 1` zooms in), keeping the position under `x`
    /// in place.
    pub fn zoom_at(&mut self, x: f64, factor: f64) {
        if !factor.is_finite() || factor <= 0.0 || !x.is_finite() {
            return;
        }
        let anchor = self.pos_at(x);
        self.per_px = (self.per_px * factor).clamp(min_per_px(), self.max_per_px());
        self.left = anchor - x * self.per_px;
        self.clamp_left();
    }

    /// Scrolls by `dx` pixels (positive: later frames come into view).
    pub fn pan(&mut self, dx: f64) {
        if dx.is_finite() {
            self.left += dx * self.per_px;
            self.clamp_left();
        }
    }

    /// Frame position at `x`.
    pub fn pos_at(&self, x: f64) -> f64 {
        self.left + x * self.per_px
    }

    /// `x` of frame position `pos`.
    pub fn x_of(&self, pos: f64) -> f64 {
        (pos - self.left) / self.per_px
    }

    /// The frame under `x`, clamped to the clip.
    pub fn frame_at(&self, x: f64) -> i64 {
        let last = (self.frames - 1.0).max(0.0);
        // Float → int casts saturate, NaN becomes 0.
        self.pos_at(x).floor().clamp(0.0, last) as i64
    }

    /// The frame boundary nearest to `x` (`0..=frames`), for range handles.
    pub fn boundary_at(&self, x: f64) -> i64 {
        self.pos_at(x).round().clamp(0.0, self.frames) as i64
    }

    /// Scrolls so that `frame` is visible; if it is not, it ends up in the middle.
    pub fn ensure_visible(&mut self, frame: i64) {
        let pos = frame as f64;
        if self.x_of(pos) < 0.0 || self.x_of(pos + 1.0) > self.width {
            self.left = pos + 0.5 - self.width * self.per_px / 2.0;
            self.clamp_left();
        }
    }

    fn max_per_px(&self) -> f64 {
        (self.frames / self.width).max(min_per_px())
    }

    fn clamp_left(&mut self) {
        let max_left = (self.frames - self.width * self.per_px).max(0.0);
        self.left = if self.left.is_finite() {
            self.left.clamp(0.0, max_left)
        } else {
            0.0
        };
    }
}

fn min_per_px() -> f64 {
    1.0 / MAX_PX_PER_FRAME
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(frames: i64, width: f64) -> View {
        let mut view = View::new(frames);
        view.set_width(width);
        view
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn starts_fitted_and_stays_fitted_on_resize() {
        let mut v = view(1000, 500.0);
        assert!(v.is_fitted());
        assert!(close(v.frames_per_px(), 2.0));
        v.set_width(1000.0);
        assert!(close(v.frames_per_px(), 1.0));
        assert!(close(v.right(), 1000.0));
    }

    #[test]
    fn zoom_keeps_the_position_under_the_pointer() {
        let mut v = view(10_000, 1000.0);
        let before = v.pos_at(300.0);
        v.zoom_at(300.0, 0.1);
        assert!(close(v.pos_at(300.0), before));
        assert!(close(v.frames_per_px(), 1.0));
        assert!(!v.is_fitted());
    }

    #[test]
    fn zoom_is_limited_both_ways() {
        let mut v = view(10_000, 1000.0);
        v.zoom_at(0.0, 1e-12);
        assert!(close(v.frames_per_px(), 1.0 / MAX_PX_PER_FRAME));
        v.zoom_at(0.0, 1e12);
        assert!(v.is_fitted());
        assert!(close(v.left(), 0.0));
    }

    #[test]
    fn short_clip_cannot_zoom_past_the_limit() {
        // 3 frames in 1000 px would be 333 px per frame; the limit is 120.
        let v = view(3, 1000.0);
        assert!(close(v.frames_per_px(), 1.0 / MAX_PX_PER_FRAME));
        assert!(close(v.x_of(v.end()), 360.0));
        assert_eq!(v.frame_at(900.0), 2);
    }

    #[test]
    fn pan_stays_inside_the_clip() {
        let mut v = view(10_000, 1000.0);
        v.zoom_at(0.0, 0.1); // 1000 frames visible
        v.pan(-50.0);
        assert!(close(v.left(), 0.0));
        v.pan(1e9);
        assert!(close(v.left(), 9000.0));
        assert!(close(v.right(), 10_000.0));
    }

    #[test]
    fn frame_and_boundary_under_the_pointer() {
        let v = view(100, 1000.0); // 10 px per frame
        assert_eq!(v.frame_at(0.0), 0);
        assert_eq!(v.frame_at(19.9), 1);
        assert_eq!(v.frame_at(-5.0), 0);
        assert_eq!(v.frame_at(5000.0), 99);
        assert_eq!(v.boundary_at(14.0), 1);
        assert_eq!(v.boundary_at(16.0), 2);
        assert_eq!(v.boundary_at(5000.0), 100);
        assert_eq!(v.frame_at(f64::NAN), 0);
    }

    #[test]
    fn ensure_visible_centres_a_hidden_frame() {
        let mut v = view(10_000, 1000.0);
        v.zoom_at(0.0, 0.1); // frames 0..1000 visible
        v.ensure_visible(500);
        assert!(close(v.left(), 0.0), "visible frames do not scroll");
        v.ensure_visible(5000);
        assert!(close(v.pos_at(500.0), 5000.5));
        v.ensure_visible(9999);
        assert!(close(v.right(), 10_000.0));
    }

    #[test]
    fn hostile_numbers_are_ignored() {
        let mut v = view(100, 1000.0);
        let before = v;
        v.zoom_at(10.0, f64::NAN);
        v.zoom_at(10.0, 0.0);
        v.zoom_at(f64::INFINITY, 0.5);
        v.pan(f64::NAN);
        assert_eq!(v, before);
        v.set_width(f64::NAN);
        assert!(close(v.width(), 1.0));
        let empty = view(0, 100.0);
        assert_eq!(empty.frame_at(50.0), 0);
    }
}
