//! [`MediaBackend`] implementation that runs `ffmpeg`/`ffprobe` as separate processes.

use std::ffi::OsStr;
use std::path::{self, Path, PathBuf};
use std::time::Duration;

use microcut_time::DecimalSecs;

use crate::frame::{self, fit_within, rgba_len};
use crate::locate::{find_tool, search_dirs};
use crate::process::{self, Limits, Streaming};
use crate::stream::{self, FfmpegAudioStream, FfmpegVideoStream};
use crate::{
    AudioFormat, AudioStream, MediaBackend, MediaError, MediaInfo, VideoFrame, VideoStream, probe,
};

const PROBE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(30),
    max_stdout: 16 * 1024 * 1024,
    max_stderr: 64 * 1024,
};

/// Seeking decodes from the previous keyframe, which can be seconds of video.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

/// Media backend using the `ffmpeg` and `ffprobe` programs.
#[derive(Debug, Clone)]
pub struct FfmpegBackend {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl FfmpegBackend {
    /// Finds `ffmpeg` and `ffprobe` (see [`crate::FFMPEG_DIR_ENV`] for the search order).
    pub fn locate() -> Result<FfmpegBackend, MediaError> {
        let dirs = search_dirs();
        let find =
            |tool: &'static str| find_tool(tool, &dirs).ok_or(MediaError::ToolNotFound { tool });
        Ok(FfmpegBackend {
            ffmpeg: find("ffmpeg")?,
            ffprobe: find("ffprobe")?,
        })
    }

    /// Path of the `ffmpeg` executable.
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg
    }

    /// Path of the `ffprobe` executable.
    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe
    }
}

impl MediaBackend for FfmpegBackend {
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        // An absolute path can never be mistaken for a command-line option ("-…").
        let path = path::absolute(path).map_err(|_| MediaError::FileNotFound(path.into()))?;
        if !path.is_file() {
            return Err(MediaError::FileNotFound(path));
        }

        let options = [
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ];
        let args = options.iter().map(OsStr::new).chain([path.as_os_str()]);
        let output = process::run("ffprobe", &self.ffprobe, args, &PROBE_LIMITS)?;
        if !output.status.success() {
            return Err(tool_failed("ffprobe", &path, &output.stderr));
        }
        probe::parse_probe(&output.stdout, path)
    }

    fn decode_frame(
        &self,
        media: &MediaInfo,
        frame: i64,
        max_size: (u32, u32),
    ) -> Result<VideoFrame, MediaError> {
        let frame = frame::clamp_frame(media, frame);
        let (width, height) = fit_within(media.video.display_size(), max_size);
        let expected = rgba_len(width, height).ok_or(MediaError::BadFrame {
            width,
            height,
            got: 0,
        })?;

        let seek = DecimalSecs(frame::seek_target(media, frame)).to_string();
        let scale = format!("scale={width}:{height}:flags=bilinear");
        // `-ss` before `-i` seeks in the input; ffmpeg then decodes from the previous keyframe
        // and drops frames before the target, so the first frame out is exactly `frame`.
        // `0:V:0` is the first real video stream (no cover pictures); rotation is applied
        // automatically before `scale`.
        let args = [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-ss"),
            OsStr::new(&seek),
            OsStr::new("-i"),
            media.path.as_os_str(),
            OsStr::new("-map"),
            OsStr::new("0:V:0"),
            OsStr::new("-frames:v"),
            OsStr::new("1"),
            OsStr::new("-vf"),
            OsStr::new(&scale),
            OsStr::new("-pix_fmt"),
            OsStr::new("rgba"),
            OsStr::new("-f"),
            OsStr::new("rawvideo"),
            OsStr::new("pipe:1"),
        ];
        let limits = Limits {
            timeout: FRAME_TIMEOUT,
            max_stdout: u64::try_from(expected)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
            max_stderr: 64 * 1024,
        };
        let output = process::run("ffmpeg", &self.ffmpeg, args, &limits)?;
        if !output.status.success() {
            return Err(tool_failed("ffmpeg", &media.path, &output.stderr));
        }
        if output.stdout.is_empty() {
            return Err(MediaError::NoFrame { frame });
        }
        let got = output.stdout.len();
        VideoFrame::new(width, height, output.stdout).ok_or(MediaError::BadFrame {
            width,
            height,
            got,
        })
    }

    fn video_stream(
        &self,
        media: &MediaInfo,
        from: i64,
        count: i64,
        max_size: (u32, u32),
    ) -> Result<Box<dyn VideoStream>, MediaError> {
        let from = frame::clamp_frame(media, from);
        let count = count.clamp(1, media.frame_count().saturating_sub(from).max(1));
        let (width, height) = fit_within(media.video.display_size(), max_size);
        let frame_len = rgba_len(width, height).ok_or(MediaError::BadFrame {
            width,
            height,
            got: 0,
        })?;

        let seek = DecimalSecs(frame::seek_target(media, from)).to_string();
        let count_arg = count.to_string();
        let scale = format!("scale={width}:{height}:flags=bilinear");
        // As in `decode_frame`; `passthrough` makes every decoded picture one output picture
        // (no duplicates or drops), so picture n of the stream is frame `from + n`.
        let args = [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-ss"),
            OsStr::new(&seek),
            OsStr::new("-i"),
            media.path.as_os_str(),
            OsStr::new("-map"),
            OsStr::new("0:V:0"),
            OsStr::new("-frames:v"),
            OsStr::new(&count_arg),
            OsStr::new("-fps_mode"),
            OsStr::new("passthrough"),
            OsStr::new("-vf"),
            OsStr::new(&scale),
            OsStr::new("-pix_fmt"),
            OsStr::new("rgba"),
            OsStr::new("-f"),
            OsStr::new("rawvideo"),
            OsStr::new("pipe:1"),
        ];
        let process = Streaming::spawn("ffmpeg", &self.ffmpeg, args, stream::MAX_STDERR)?;
        Ok(Box::new(FfmpegVideoStream {
            process,
            path: media.path.clone(),
            size: (width, height),
            frame_len,
            ended: false,
        }))
    }

    fn audio_stream(
        &self,
        media: &MediaInfo,
        track: usize,
        from: i64,
        count: i64,
        format: AudioFormat,
    ) -> Result<Box<dyn AudioStream>, MediaError> {
        if track >= media.audio.len() {
            return Err(MediaError::NoAudioStream);
        }
        let map_arg = format!("0:a:{track}");
        let AudioFormat {
            sample_rate,
            channels,
        } = format;
        if !(1..=768_000).contains(&sample_rate) || !(1..=64).contains(&channels) {
            return Err(MediaError::BadAudioFormat {
                sample_rate,
                channels,
            });
        }
        let rate = media.video.frame_rate;
        let from = frame::clamp_frame(media, from);
        let start = rate.frame_to_tick(from);
        let end = rate.frame_to_tick(from.saturating_add(count.max(0)));
        let start_arg = DecimalSecs(start).to_string();
        let duration_arg = DecimalSecs(end.saturating_sub(start)).to_string();
        let (channels_arg, rate_arg) = (channels.to_string(), sample_rate.to_string());
        // Audio seeks are sample-exact: ffmpeg decodes from before `-ss` and trims.
        let args = [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-ss"),
            OsStr::new(&start_arg),
            OsStr::new("-i"),
            media.path.as_os_str(),
            OsStr::new("-map"),
            OsStr::new(&map_arg),
            OsStr::new("-vn"),
            OsStr::new("-t"),
            OsStr::new(&duration_arg),
            OsStr::new("-ac"),
            OsStr::new(&channels_arg),
            OsStr::new("-ar"),
            OsStr::new(&rate_arg),
            OsStr::new("-f"),
            OsStr::new("f32le"),
            OsStr::new("pipe:1"),
        ];
        let process = Streaming::spawn("ffmpeg", &self.ffmpeg, args, stream::MAX_STDERR)?;
        Ok(Box::new(FfmpegAudioStream {
            process,
            path: media.path.clone(),
            bytes: Vec::new(),
            ended: false,
            pending_error: None,
        }))
    }
}

/// Error from the last stderr line. ffmpeg tools prefix errors with the input path, which the UI
/// already shows, so that prefix is removed.
pub(crate) fn tool_failed(tool: &'static str, path: &Path, stderr: &[u8]) -> MediaError {
    let line = process::last_line(stderr);
    let prefix = format!("{}: ", path.display());
    let message = line.strip_prefix(&prefix).unwrap_or(&line).to_owned();
    MediaError::ToolFailed { tool, message }
}
