//! Decoded pictures and the frame arithmetic around them (pure, no ffmpeg).

use microcut_time::Tick;

use crate::MediaInfo;

/// Largest edge of a decoded picture. Keeps allocations bounded and below GPU texture limits.
pub const MAX_FRAME_EDGE: u32 = 4096;

/// One decoded picture, in display orientation (rotation already applied).
#[derive(Clone, PartialEq, Eq)]
pub struct VideoFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl VideoFrame {
    /// Wraps RGBA bytes; `None` unless `rgba.len() == width * height * 4` and both sides are
    /// non-zero. So every `VideoFrame` is safe to hand to a texture upload.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<VideoFrame> {
        let expected = rgba_len(width, height)?;
        (width > 0 && height > 0 && rgba.len() == expected).then_some(VideoFrame {
            width,
            height,
            rgba,
        })
    }

    /// Width in pixels; never zero.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels; never zero.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixels, row by row, 4 bytes (R, G, B, A) each, not premultiplied.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VideoFrame({}x{})", self.width, self.height)
    }
}

/// `width * height * 4`, if it fits in `usize`.
pub(crate) fn rgba_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
}

/// Largest size with the aspect ratio of `src` that fits in `max` (and in [`MAX_FRAME_EDGE`]),
/// never larger than `src`, never zero.
pub(crate) fn fit_within(src: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (u64::from(src.0.max(1)), u64::from(src.1.max(1)));
    let mw = u64::from(max.0.clamp(1, MAX_FRAME_EDGE)).min(sw);
    let mh = u64::from(max.1.clamp(1, MAX_FRAME_EDGE)).min(sh);
    // Width-limited unless that makes the height too large.
    let (w, h) = if sh * mw <= mh * sw {
        (mw, sh * mw / sw)
    } else {
        (sw * mh / sh, mh)
    };
    let clamp = |v: u64| {
        u32::try_from(v)
            .unwrap_or(MAX_FRAME_EDGE)
            .clamp(1, MAX_FRAME_EDGE)
    };
    (clamp(w), clamp(h))
}

/// `frame` clamped to the frames of `media` (0 ..= last).
pub fn clamp_frame(media: &MediaInfo, frame: i64) -> i64 {
    frame.clamp(0, media.frame_count().saturating_sub(1).max(0))
}

/// Where to seek (relative to the start of the media) so that the first frame ffmpeg delivers is
/// `frame`: half a frame before it. Container timestamps are rounded (Matroska: whole
/// milliseconds), so seeking exactly to the frame's start could skip it.
pub(crate) fn seek_target(media: &MediaInfo, frame: i64) -> Tick {
    let rate = media.video.frame_rate;
    let start = rate.frame_to_tick(frame);
    let next = rate.frame_to_tick(frame.saturating_add(1));
    let half_frame = Tick(next.saturating_sub(start).0 / 2);
    start.saturating_sub(half_frame).max(Tick::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VideoInfo;
    use microcut_time::{FrameRate, TICKS_PER_SECOND};
    use std::path::PathBuf;

    fn media(fps: u32, secs: i64) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("x.mkv"),
            container: String::new(),
            start: Tick::ZERO,
            duration: Tick::from_secs(secs),
            video: VideoInfo {
                codec: String::new(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(fps, 1).unwrap(),
                rotation: 0,
            },
            audio: None,
        }
    }

    #[test]
    fn fits_keeping_aspect_and_never_upscales() {
        assert_eq!(fit_within((1280, 720), (640, 640)), (640, 360));
        assert_eq!(fit_within((1280, 720), (1000, 200)), (355, 200));
        assert_eq!(fit_within((320, 240), (4000, 4000)), (320, 240));
        assert_eq!(fit_within((1080, 1920), (800, 600)), (337, 600));
    }

    #[test]
    fn fit_survives_degenerate_sizes() {
        assert_eq!(fit_within((0, 0), (0, 0)), (1, 1));
        assert_eq!(fit_within((10_000, 1), (300, 300)), (300, 1));
        assert_eq!(
            fit_within((u32::MAX, u32::MAX), (u32::MAX, u32::MAX)),
            (4096, 4096)
        );
    }

    #[test]
    fn frame_requires_exact_buffer() {
        assert!(VideoFrame::new(2, 2, vec![0; 16]).is_some());
        assert!(VideoFrame::new(2, 2, vec![0; 15]).is_none());
        assert!(VideoFrame::new(0, 2, vec![]).is_none());
        assert!(VideoFrame::new(u32::MAX, u32::MAX, vec![]).is_none());
    }

    #[test]
    fn frames_clamp_to_media() {
        let m = media(60, 2);
        assert_eq!(clamp_frame(&m, -5), 0);
        assert_eq!(clamp_frame(&m, 119), 119);
        assert_eq!(clamp_frame(&m, 120), 119);
        assert_eq!(clamp_frame(&m, i64::MAX), 119);
    }

    #[test]
    fn seek_lands_half_a_frame_early() {
        let m = media(60, 2);
        assert_eq!(seek_target(&m, 0), Tick::ZERO);
        let frame_len = TICKS_PER_SECOND / 60;
        assert_eq!(seek_target(&m, 1), Tick(frame_len / 2));
        assert_eq!(seek_target(&m, 30), Tick(30 * frame_len - frame_len / 2));
    }
}
