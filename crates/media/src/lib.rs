//! Media backend for MicroCut (layer L1).
//!
//! [`MediaBackend`] is what the rest of MicroCut talks to; [`FfmpegBackend`] implements it by
//! running `ffmpeg`/`ffprobe` as separate processes (see `docs/adr/0001-ffmpeg-backend.md`).
//! No other crate spawns ffmpeg or parses its output.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod ffmpeg;
mod info;
mod locate;
mod probe;
mod process;

use std::path::Path;

pub use error::MediaError;
pub use ffmpeg::FfmpegBackend;
pub use info::{AudioInfo, MediaInfo, VideoInfo};
pub use locate::FFMPEG_DIR_ENV;

/// Everything MicroCut needs from a media library. Implementations must be usable from worker
/// threads and must never panic on bad input.
pub trait MediaBackend: Send + Sync {
    /// Reads container and stream information of the video at `path`.
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError>;
}
