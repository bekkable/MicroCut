//! Media backend for MicroCut (layer L1).
//!
//! [`MediaBackend`] is what the rest of MicroCut talks to; [`FfmpegBackend`] implements it by
//! running `ffmpeg`/`ffprobe` as separate processes (see `docs/adr/0001-ffmpeg-backend.md`).
//! No other crate spawns ffmpeg or parses its output.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod export;
mod ffmpeg;
mod frame;
mod info;
mod locate;
mod probe;
mod process;
mod stream;

use std::path::Path;

use microcut_time::Tick;

pub use error::MediaError;
pub use export::{
    AudioCodec, AudioExport, Encoders, ExportEnd, ExportJob, VideoCodec, VideoQuality,
};
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

    /// Decodes up to `count` consecutive frames starting at frame `from` (clamped to the media),
    /// scaled like [`MediaBackend::decode_frame`]. Every picture of the stream is one frame
    /// index; pictures are produced as fast as they are read.
    fn video_stream(
        &self,
        media: &MediaInfo,
        from: i64,
        count: i64,
        max_size: (u32, u32),
    ) -> Result<Box<dyn VideoStream>, MediaError>;

    /// Decodes audio track `track` (index into [`MediaInfo::audio`]) for frames `span` =
    /// `(from, count)`: from the start of frame `from` for the duration of `count` frames,
    /// converted to `format` (interleaved `f32` samples).
    ///
    /// `delay` shifts the sound against the picture (audio sync): with a positive delay the
    /// stream plays what the source has `delay` earlier, so the sound comes later; before the
    /// start of the source that is silence. A negative delay makes the sound come earlier.
    fn audio_stream(
        &self,
        media: &MediaInfo,
        track: usize,
        span: (i64, i64),
        delay: Tick,
        format: AudioFormat,
    ) -> Result<Box<dyn AudioStream>, MediaError>;

    /// The export encoders the installed media library offers.
    fn encoders(&self) -> Result<Encoders, MediaError>;

    /// Encodes `job` (blocking until done). The output file is created first and must not exist;
    /// it is removed again if the export fails or is cancelled. `progress` is called with the
    /// number of frames written so far, a few times per second; returning `false` cancels.
    /// Video frames are exact: the first frame of the output is frame `job.from`.
    fn export(
        &self,
        media: &MediaInfo,
        job: &ExportJob,
        progress: &mut dyn FnMut(i64) -> bool,
    ) -> Result<ExportEnd, MediaError>;
}

/// Consecutive decoded frames (see [`MediaBackend::video_stream`]). Dropping it stops decoding.
pub trait VideoStream: Send {
    /// Size of every picture of the stream.
    fn size(&self) -> (u32, u32);

    /// The next picture; `None` after the last one. Blocks until it is decoded.
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, MediaError>;
}

/// Decoded audio samples (see [`MediaBackend::audio_stream`]). Dropping it stops decoding.
pub trait AudioStream: Send {
    /// Fills `out` with interleaved samples and returns how many it wrote; fewer than requested
    /// only at the end, `0` after the end. Blocks until the samples are decoded.
    fn read(&mut self, out: &mut [f32]) -> Result<usize, MediaError>;
}

/// Sample format for [`MediaBackend::audio_stream`] (what the sound device wants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Samples per second and channel.
    pub sample_rate: u32,
    /// Interleaved channels.
    pub channels: u16,
}
