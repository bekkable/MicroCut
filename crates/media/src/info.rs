use std::path::PathBuf;

use microcut_time::{FrameRate, Tick};

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
    /// The first audio stream, if any.
    pub audio: Option<AudioInfo>,
}

impl MediaInfo {
    /// Number of whole frames, derived from duration and frame rate.
    pub fn frame_count(&self) -> i64 {
        self.video.frame_rate.tick_to_frame(self.duration)
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

/// The audio stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// Codec name, e.g. `aac`.
    pub codec: String,
    /// Sample rate in Hz, if known.
    pub sample_rate: Option<u32>,
    /// Number of channels, if known.
    pub channels: Option<u32>,
}
