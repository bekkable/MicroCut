use std::path::PathBuf;

use crate::locate::FFMPEG_DIR_ENV;

/// Errors from the media backend. The `Display` text is shown to the user as is.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// `ffmpeg` or `ffprobe` could not be found.
    #[error(
        "{tool} was not found. Install FFmpeg and add it to PATH, put {tool} next to MicroCut, \
         or set {FFMPEG_DIR_ENV} to the folder that contains it."
    )]
    ToolNotFound {
        /// Name of the missing program.
        tool: &'static str,
    },

    /// The input file does not exist or is not a regular file.
    #[error("file not found: {}", .0.display())]
    FileNotFound(PathBuf),

    /// The program exists but could not be started.
    #[error("could not run {tool}: {source}")]
    Spawn {
        /// Name of the program.
        tool: &'static str,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// The program did not finish in time and was stopped.
    #[error("{tool} did not finish within {secs} s")]
    Timeout {
        /// Name of the program.
        tool: &'static str,
        /// Time limit in seconds.
        secs: u64,
    },

    /// The program reported an error (usually: not a media file, or damaged).
    #[error("{tool} could not read the file: {message}")]
    ToolFailed {
        /// Name of the program.
        tool: &'static str,
        /// Last line of its error output.
        message: String,
    },

    /// `ffprobe` produced output MicroCut does not understand.
    #[error("unexpected output from ffprobe: {0}")]
    InvalidProbe(String),

    /// The file has no (usable) video stream.
    #[error("the file contains no video stream")]
    NoVideoStream,

    /// Neither the container nor the video stream states a duration.
    #[error("the duration of the video is unknown")]
    UnknownDuration,

    /// Audio was requested from a track the file does not have.
    #[error("the file contains no such audio track")]
    NoAudioStream,

    /// An audio format no device would ask for (zero rate or channels).
    #[error("unsupported audio format: {sample_rate} Hz, {channels} channels")]
    BadAudioFormat {
        /// Requested sample rate.
        sample_rate: u32,
        /// Requested channel count.
        channels: u16,
    },

    /// The decoder delivered no picture for this frame (e.g. past the end of the video stream).
    #[error("no picture at frame {frame}")]
    NoFrame {
        /// The requested frame index.
        frame: i64,
    },

    /// `ffmpeg` delivered a picture of the wrong size.
    #[error("ffmpeg returned {got} bytes for a {width}×{height} picture")]
    BadFrame {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
        /// Bytes received.
        got: usize,
    },
}
