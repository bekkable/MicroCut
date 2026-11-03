//! [`MediaBackend`] implementation that runs `ffmpeg`/`ffprobe` as separate processes.

use std::ffi::OsStr;
use std::path::{self, Path, PathBuf};
use std::time::Duration;

use crate::locate::{find_tool, search_dirs};
use crate::process::{self, Limits};
use crate::{MediaBackend, MediaError, MediaInfo, probe};

const PROBE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(30),
    max_stdout: 16 * 1024 * 1024,
    max_stderr: 64 * 1024,
};

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
            let line = process::last_line(&output.stderr);
            // ffprobe prefixes errors with the input path, which the UI already shows.
            let prefix = format!("{}: ", path.display());
            let message = line.strip_prefix(&prefix).unwrap_or(&line).to_owned();
            return Err(MediaError::ToolFailed {
                tool: "ffprobe",
                message,
            });
        }
        probe::parse_probe(&output.stdout, path)
    }
}
