//! Thumbnail tiles of the video strip.
//!
//! Each tile shows its first frame. Tiles span 1, 2, 3, 4, 6, 8, 12, 16, … frames (powers of two
//! and 1.5 × powers of two), so a tile is at most one and a half thumbnails wide (from two frames
//! per tile on) and the
//! frames shown stay the same while scrolling and recur at neighbouring zoom levels — decoded
//! thumbnails can be reused.

use crate::view::View;

/// Never lay out more tiles than this.
const MAX_TILES: usize = 1_000;

/// One tile of the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tile {
    /// Frame shown on the tile (its first frame).
    pub frame: i64,
    /// Left edge, in the view's pixels.
    pub x0: f64,
    /// Right edge (clipped to the end of the clip).
    pub x1: f64,
}

/// Tiles covering the visible part of the clip for thumbnails `thumb_px` wide.
pub fn tiles(view: &View, thumb_px: f64) -> Vec<Tile> {
    let frames_per_tile = frames_per_tile(view.frames_per_px() * thumb_px);
    let span = frames_per_tile as f64;
    let first = (view.left() / span).floor().max(0.0) as i64;
    let end = view.end();
    (first..)
        .take(MAX_TILES)
        .map_while(|n| {
            let frame = n.checked_mul(frames_per_tile)?;
            let pos = frame as f64;
            let x0 = view.x_of(pos);
            (pos < end && x0 < view.width()).then(|| Tile {
                frame,
                x0,
                x1: view.x_of((pos + span).min(end)),
            })
        })
        .collect()
}

/// Smallest of 1, 2, 3, 4, 6, 8, 12, … that is at least `needed`.
fn frames_per_tile(needed: f64) -> i64 {
    if !needed.is_finite() || needed <= 1.0 {
        return 1;
    }
    let mut power: i64 = 1;
    loop {
        if power as f64 >= needed {
            return power;
        }
        let between = power + power / 2;
        if power >= 2 && between as f64 >= needed {
            return between;
        }
        match power.checked_mul(2) {
            Some(next) => power = next,
            None => return power,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_spans_follow_the_sequence() {
        let spans: Vec<i64> = [0.5, 1.0, 1.5, 2.5, 3.5, 5.0, 7.0, 9.0, 13.0]
            .into_iter()
            .map(frames_per_tile)
            .collect();
        assert_eq!(spans, [1, 1, 2, 3, 4, 6, 8, 12, 16]);
        assert_eq!(frames_per_tile(f64::NAN), 1);
        assert_eq!(frames_per_tile(1e300), 1 << 62);
    }

    #[test]
    fn tiles_are_at_most_one_and_a_half_thumbnails_wide() {
        // From two frames on; below that the only choices are one and two frames.
        for needed in [2.0, 2.9, 3.1, 5.5, 100.0, 12345.6] {
            let span = frames_per_tile(needed) as f64;
            assert!(
                span >= needed && span < needed * 1.5 + 1e-9,
                "{needed} → {span}"
            );
        }
    }

    #[test]
    fn tiles_cover_the_visible_clip() {
        let mut view = View::new(1000);
        view.set_width(500.0); // 2 frames per px
        view.zoom_at(0.0, 0.5); // 1 frame per px, frames 0..500 visible
        view.pan(130.0);
        let tiles = tiles(&view, 100.0); // 100 frames needed → 128 per tile
        assert_eq!(tiles.first().map(|t| t.frame), Some(128));
        assert!(tiles.first().unwrap().x0 <= 0.0);
        assert!(tiles.last().unwrap().x1 >= 500.0);
        for pair in tiles.windows(2) {
            assert_eq!(pair[1].frame - pair[0].frame, 128);
            assert!((pair[0].x1 - pair[1].x0).abs() < 1e-9);
        }
    }

    #[test]
    fn last_tile_ends_with_the_clip() {
        let mut view = View::new(250);
        view.set_width(1000.0); // 4 px per frame
        let tiles = tiles(&view, 90.0); // 22.5 frames → 24 per tile
        let last = tiles.last().unwrap();
        assert_eq!(last.frame, 240);
        assert!((last.x1 - 1000.0).abs() < 1e-9);
        assert_eq!(tiles.len(), 11);
    }
}
