//! Media backend for MicroCut (layer L1).
//!
//! [`MediaBackend`] is what the rest of MicroCut talks to; [`FfmpegBackend`] implements it by
//! running `ffmpeg`/`ffprobe` as separate processes (see `docs/adr/0001-ffmpeg-backend.md`).
//! No other crate spawns ffmpeg or parses its output.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod ffmpeg;
mod frame;
mod info;
mod locate;
mod probe;
mod process;

use std::path::Path;

pub use error::MediaError;
pub use ffmpeg::FfmpegBackend;
pub use frame::{MAX_FRAME_EDGE, VideoFrame, clamp_frame};
pub use info::{AudioInfo, MediaInfo, VideoInfo};
pub use locate::FFMPEG_DIR_ENV;

/// Everything MicroCut needs from a media library. Implementations must be usable from worker
/// threads and must never panic on bad input.
pub trait MediaBackend: Send + Sync {
    /// Reads container and stream information of the video at `path`.
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError>;

    /// Decodes frame `frame` (clamped to the media) of `media`, scaled to fit in `max_size`
    /// (width, height) with the display aspect ratio kept and rotation applied. Never upscales.
    fn decode_frame(
        &self,
        media: &MediaInfo,
        frame: i64,
        max_size: (u32, u32),
    ) -> Result<VideoFrame, MediaError>;
}
