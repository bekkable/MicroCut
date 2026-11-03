//! Playback from the cursor: a video stream and (if there is sound) an audio stream from ffmpeg,
//! each read by a worker thread; the UI shows whichever frame the clock has reached.
//!
//! The clock is the sound device (samples actually played) so picture and sound stay together;
//! without sound, or after the sound ended, it is the system clock. It starts when the first
//! picture is there, so ffmpeg's start-up time never makes the video jump.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;
use std::time::Instant;

use eframe::egui;
use microcut_media::{AudioFormat, MediaBackend, MediaInfo, VideoFrame};

use crate::audio::{AudioOut, AudioShared};

/// Decoded pictures waiting to be shown.
const VIDEO_BUFFER: usize = 6;

/// Samples per read from the audio decoder.
const AUDIO_CHUNK: usize = 4096;

/// What the UI should do after [`Playback::update`].
pub enum Step {
    /// Show this frame (index, picture).
    Show(i64, VideoFrame),
    /// Nothing new yet.
    Wait,
    /// Played to the end.
    Finished,
    /// Stopped by an error.
    Failed(String),
}

/// Where the time comes from.
enum Clock {
    /// Not started: waiting for the first picture.
    Waiting,
    /// Sound device clock.
    Audio,
    /// System clock: seconds at `since` plus the time elapsed since then.
    Wall { since: Instant, base: f64 },
}

/// A running playback. Dropping it stops the workers and ffmpeg.
pub struct Playback {
    from: i64,
    fps: f64,
    frames: Receiver<Result<VideoFrame, String>>,
    video_stop: Arc<AtomicBool>,
    /// Index of the next picture to arrive.
    next: i64,
    pending: Option<(i64, VideoFrame)>,
    stream_ended: bool,
    clock: Clock,
    audio: Option<AudioOut>,
    /// Why there is no sound (shown to the user), if there is none.
    audio_note: Option<String>,
}

impl Playback {
    /// Starts playing frames `from..=last` of `media`, pictures scaled to fit `size`.
    pub fn start(
        backend: &Arc<dyn MediaBackend>,
        media: &Arc<MediaInfo>,
        (from, last): (i64, i64),
        size: (u32, u32),
        ctx: &egui::Context,
    ) -> Playback {
        let count = last.saturating_sub(from).saturating_add(1).max(1);
        let rate = media.video.frame_rate;

        let (tx, frames) = mpsc::sync_channel(VIDEO_BUFFER);
        let video_stop = Arc::new(AtomicBool::new(false));
        let video_started = {
            let (backend, media, stop, ctx) = (
                Arc::clone(backend),
                Arc::clone(media),
                Arc::clone(&video_stop),
                ctx.clone(),
            );
            thread::Builder::new()
                .name("microcut-play-video".to_owned())
                .spawn(move || {
                    let job = || {
                        video_worker(
                            backend.as_ref(),
                            &media,
                            (from, count),
                            size,
                            &tx,
                            &stop,
                            &ctx,
                        )
                    };
                    if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
                        // The receiver may be gone already; nothing else to tell.
                        let _ = tx.try_send(Err("internal error during playback".to_owned()));
                    }
                })
                .is_ok()
        };

        let (audio, audio_note) = match &media.audio {
            None => (None, None),
            Some(_) => match start_audio(backend, media, (from, count)) {
                Ok(out) => (Some(out), None),
                Err(note) => (None, Some(note)),
            },
        };

        let mut playback = Playback {
            from,
            fps: f64::from(rate.num()) / f64::from(rate.den()),
            frames,
            video_stop,
            next: from,
            pending: None,
            stream_ended: !video_started,
            clock: Clock::Waiting,
            audio,
            audio_note,
        };
        if !video_started {
            playback.audio = None;
        }
        playback
    }

    /// Why playback is silent, if it is.
    pub fn audio_note(&self) -> Option<&str> {
        self.audio_note.as_deref()
    }

    /// Advances to the frame the clock has reached.
    pub fn update(&mut self) -> Step {
        if self.pending.is_none()
            && let Err(message) = self.receive()
        {
            return Step::Failed(message);
        }
        if matches!(self.clock, Clock::Waiting) && self.pending.is_some() {
            self.start_clock();
        }
        let target = self
            .from
            .saturating_add(frames_at(self.elapsed(), self.fps));

        let mut newest = None;
        while let Some((index, _)) = &self.pending {
            if *index > target {
                break;
            }
            newest = self.pending.take();
            if let Err(message) = self.receive() {
                return Step::Failed(message);
            }
        }
        match newest {
            Some((index, frame)) => Step::Show(index, frame),
            None if self.stream_ended && self.pending.is_none() => Step::Finished,
            None => Step::Wait,
        }
    }

    /// Takes the next picture into `pending`, if one is ready.
    fn receive(&mut self) -> Result<(), String> {
        match self.frames.try_recv() {
            Ok(Ok(frame)) => {
                self.pending = Some((self.next, frame));
                self.next = self.next.saturating_add(1);
                Ok(())
            }
            Ok(Err(message)) => {
                self.stream_ended = true;
                Err(message)
            }
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                self.stream_ended = true;
                Ok(())
            }
        }
    }

    fn start_clock(&mut self) {
        self.clock = match &self.audio {
            Some(out) => {
                out.shared().start();
                Clock::Audio
            }
            None => Clock::Wall {
                since: Instant::now(),
                base: 0.0,
            },
        };
    }

    /// Seconds played since the first frame.
    fn elapsed(&mut self) -> f64 {
        match self.clock {
            Clock::Waiting => 0.0,
            Clock::Wall { since, base } => base + since.elapsed().as_secs_f64(),
            Clock::Audio => {
                let Some(out) = &self.audio else {
                    return 0.0;
                };
                let secs = out.shared().played_secs(out.format());
                // Sound shorter than the picture, or the device failed: continue on the system
                // clock from here.
                if out.shared().exhausted() || out.shared().error().is_some() {
                    if let Some(error) = out.shared().error() {
                        self.audio_note = Some(error);
                    }
                    self.clock = Clock::Wall {
                        since: Instant::now(),
                        base: secs,
                    };
                }
                secs
            }
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.video_stop.store(true, Ordering::Release);
        if let Some(out) = &self.audio {
            out.shared().request_stop();
        }
    }
}

/// Whole frames played after `secs` at `fps` (only for pacing the display).
fn frames_at(secs: f64, fps: f64) -> i64 {
    let frames = (secs * fps).floor();
    if frames.is_finite() && frames > 0.0 {
        frames as i64
    } else {
        0
    }
}

fn video_worker(
    backend: &dyn MediaBackend,
    media: &MediaInfo,
    (from, count): (i64, i64),
    size: (u32, u32),
    tx: &SyncSender<Result<VideoFrame, String>>,
    stop: &AtomicBool,
    ctx: &egui::Context,
) {
    let mut stream = match backend.video_stream(media, from, count, size) {
        Ok(stream) => stream,
        Err(err) => {
            let _ = tx.send(Err(err.to_string()));
            return;
        }
    };
    while !stop.load(Ordering::Acquire) {
        let item = match stream.next_frame() {
            Ok(Some(frame)) => Ok(frame),
            Ok(None) => return,
            Err(err) => Err(err.to_string()),
        };
        let failed = item.is_err();
        // Blocks while the buffer is full; fails once the playback is dropped.
        if tx.send(item).is_err() || failed {
            return;
        }
        ctx.request_repaint();
    }
}

/// Opens the sound device and starts decoding into it; the reason as text if there is no
/// sound.
fn start_audio(
    backend: &Arc<dyn MediaBackend>,
    media: &Arc<MediaInfo>,
    span: (i64, i64),
) -> Result<AudioOut, String> {
    let out = AudioOut::open()?;
    let shared = Arc::clone(out.shared());
    let format = out.format();
    let (backend, media) = (Arc::clone(backend), Arc::clone(media));
    thread::Builder::new()
        .name("microcut-play-audio".to_owned())
        .spawn(move || {
            let job = || audio_worker(backend.as_ref(), &media, span, format, &shared);
            if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
                shared.set_error("internal error while playing sound".to_owned());
            }
            shared.set_decoder_done();
        })
        .map_err(|_| "could not start the sound decoder".to_owned())?;
    Ok(out)
}

fn audio_worker(
    backend: &dyn MediaBackend,
    media: &MediaInfo,
    (from, count): (i64, i64),
    format: AudioFormat,
    shared: &AudioShared,
) {
    let mut stream = match backend.audio_stream(media, from, count, format) {
        Ok(stream) => stream,
        Err(err) => {
            shared.set_error(err.to_string());
            return;
        }
    };
    let mut buf = vec![0.0; AUDIO_CHUNK];
    while !shared.stop_requested() {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if !shared.push(buf.get(..n).unwrap_or_default()) {
                    return;
                }
            }
            Err(err) => {
                shared.set_error(err.to_string());
                return;
            }
        }
    }
}

/// The frame where playback started at `cursor` stops: the end of the range if the cursor is
/// inside it (before its last frame), else the last frame of the clip.
pub fn stop_frame(cursor: i64, range: (i64, i64), last: i64) -> i64 {
    let (start, end) = range;
    if (start..end).contains(&cursor) {
        end
    } else {
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_stops_at_the_range_end_only_inside_the_range() {
        let range = (100, 200);
        assert_eq!(stop_frame(100, range, 999), 200);
        assert_eq!(stop_frame(150, range, 999), 200);
        assert_eq!(stop_frame(199, range, 999), 200);
        // On the last frame of the range or outside it: to the end of the clip.
        assert_eq!(stop_frame(200, range, 999), 999);
        assert_eq!(stop_frame(50, range, 999), 999);
        assert_eq!(stop_frame(500, range, 999), 999);
    }

    #[test]
    fn clock_to_frames() {
        let ntsc = 30_000.0 / 1_001.0;
        assert_eq!(frames_at(0.0, ntsc), 0);
        assert_eq!(frames_at(1.0, ntsc), 29);
        assert_eq!(frames_at(1.0011, ntsc), 30);
        assert_eq!(frames_at(-1.0, 60.0), 0);
        assert_eq!(frames_at(f64::NAN, 60.0), 0);
        assert_eq!(frames_at(1e300, 60.0), i64::MAX);
    }
}
