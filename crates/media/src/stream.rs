//! Video and audio streams for playback, read from a running `ffmpeg`.

use std::path::PathBuf;

use crate::process::{Fill, Streaming};
use crate::{AudioStream, MediaError, VideoFrame, VideoStream};

/// Error output kept per stream, for the message if ffmpeg fails.
pub(crate) const MAX_STDERR: u64 = 64 * 1024;

/// Raw RGBA pictures from `ffmpeg … -f rawvideo pipe:1`.
pub(crate) struct FfmpegVideoStream {
    pub(crate) process: Streaming,
    pub(crate) path: PathBuf,
    pub(crate) size: (u32, u32),
    /// Bytes per picture (`width × height × 4`).
    pub(crate) frame_len: usize,
    pub(crate) ended: bool,
}

impl VideoStream for FfmpegVideoStream {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, MediaError> {
        if self.ended {
            return Ok(None);
        }
        let (width, height) = self.size;
        let mut rgba = vec![0; self.frame_len];
        match self.process.fill(&mut rgba)? {
            Fill::Full => {
                VideoFrame::new(width, height, rgba)
                    .map(Some)
                    .ok_or(MediaError::BadFrame {
                        width,
                        height,
                        got: self.frame_len,
                    })
            }
            Fill::Ended(0) => {
                self.ended = true;
                finish(&mut self.process, &self.path)?;
                Ok(None)
            }
            Fill::Ended(got) => {
                self.ended = true;
                Err(MediaError::BadFrame { width, height, got })
            }
        }
    }
}

/// Interleaved little-endian `f32` samples from `ffmpeg … -f f32le pipe:1`.
pub(crate) struct FfmpegAudioStream {
    pub(crate) process: Streaming,
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) ended: bool,
    /// An error noticed while returning the last samples, reported on the next read.
    pub(crate) pending_error: Option<MediaError>,
}

impl AudioStream for FfmpegAudioStream {
    fn read(&mut self, out: &mut [f32]) -> Result<usize, MediaError> {
        if let Some(err) = self.pending_error.take() {
            return Err(err);
        }
        if self.ended || out.is_empty() {
            return Ok(0);
        }
        let want = out.len().saturating_mul(4);
        self.bytes.resize(want, 0);
        let Some(buf) = self.bytes.get_mut(..want) else {
            return Ok(0);
        };
        let got = match self.process.fill(buf)? {
            Fill::Full => want,
            Fill::Ended(got) => {
                self.ended = true;
                got
            }
        };
        let (samples, _) = buf.as_chunks::<4>();
        let count = got / 4;
        for (dst, bytes) in out.iter_mut().zip(samples.iter().take(count)) {
            let sample = f32::from_le_bytes(*bytes);
            *dst = if sample.is_finite() { sample } else { 0.0 };
        }
        if self.ended
            && let Err(err) = finish(&mut self.process, &self.path)
        {
            if count == 0 {
                return Err(err);
            }
            self.pending_error = Some(err);
        }
        Ok(count)
    }
}

/// Waits for ffmpeg after its output ended; its error if it failed.
fn finish(process: &mut Streaming, path: &std::path::Path) -> Result<(), MediaError> {
    let (status, stderr) = process.finish()?;
    if status.success() {
        Ok(())
    } else {
        Err(crate::ffmpeg::tool_failed("ffmpeg", path, &stderr))
    }
}
