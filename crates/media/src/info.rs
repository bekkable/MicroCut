use std::path::PathBuf;

use microcut_time::{FrameRate, TICKS_PER_SECOND, Tick};

/// Container and stream information of an opened video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Container format as reported by ffprobe, e.g. `mov,mp4,m4a,3gp,3g2,mj2`.
    pub container: String,
    /// Presentation time of the first frame (non-zero e.g. in MPEG-TS).
    pub start: Tick,
    /// Length of the media; always positive.
    pub duration: Tick,
    /// The first video stream (cover pictures are skipped).
    pub video: VideoInfo,
    /// The audio streams in file order; track `n` is ffmpeg's `0:a:n`.
    pub audio: Vec<AudioInfo>,
}

impl MediaInfo {
    /// Number of whole frames, derived from duration and frame rate.
    ///
    /// ffprobe prints durations with six decimals, so 3800 frames at 60 fps arrive as
    /// `63.333333` s, a hair short of the last frame's end; up to a microsecond short still
    /// counts as a whole frame.
    pub fn frame_count(&self) -> i64 {
        let tolerance = Tick(TICKS_PER_SECOND / 1_000_000);
        self.video
            .frame_rate
            .tick_to_frame(self.duration.saturating_add(tolerance))
    }

    /// File name for display; falls back to the full path.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}

/// The video stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInfo {
    /// Codec name, e.g. `h264`.
    pub codec: String,
    /// Coded width in pixels; never zero.
    pub width: u32,
    /// Coded height in pixels; never zero.
    pub height: u32,
    /// Frame rate (average rate, falling back to the base rate).
    pub frame_rate: FrameRate,
    /// Clockwise rotation in degrees applied on display: 0, 90, 180 or 270.
    pub rotation: u16,
}

impl VideoInfo {
    /// Width and height as displayed, i.e. after applying [`Self::rotation`].
    pub fn display_size(&self) -> (u32, u32) {
        if self.rotation % 180 == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// An audio stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// Codec name, e.g. `aac`.
    pub codec: String,
    /// Sample rate in Hz, if known.
    pub sample_rate: Option<u32>,
    /// Number of channels, if known.
    pub channels: Option<u32>,
    /// Title tag, e.g. `Mic` (OBS names its tracks), if set.
    pub title: Option<String>,
    /// Language tag, e.g. `eng`, if set and not `und`.
    pub language: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(fps: (u32, u32), duration: &str) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("clip.mp4"),
            container: "mp4".to_owned(),
            start: Tick::ZERO,
            duration: Tick::parse_decimal_secs(duration).unwrap(),
            video: VideoInfo {
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(fps.0, fps.1).unwrap(),
                rotation: 0,
            },
            audio: Vec::new(),
        }
    }

    #[test]
    fn rounded_durations_count_whole_frames() {
        // 3800 frames at 60 fps, as ffprobe prints it.
        assert_eq!(media((60, 1), "63.333333").frame_count(), 3800);
        assert_eq!(media((60, 1), "63.333334").frame_count(), 3800);
        // 1001 frames at 29.97 fps = 33.400033… s.
        assert_eq!(media((30_000, 1_001), "33.400033").frame_count(), 1001);
        // A real part of a frame does not count.
        assert_eq!(media((60, 1), "63.330000").frame_count(), 3799);
        assert_eq!(media((25, 1), "2.000000").frame_count(), 50);
        assert_eq!(media((25, 1), "0.000000").frame_count(), 0);
    }
}
