//! Frames around the paused cursor, decoded ahead in the background so that stepping frame by
//! frame shows them at once.
//!
//! While paused, the buffer keeps the frames of a [`BufferWindow`] (by default 32 to either
//! side of the cursor, `config.json`). When the cursor gets within half that distance of the
//! window's edge, the window moves to the cursor and only the frames it did not hold yet are
//! decoded. A worker thread decodes each missing run of frames with one ffmpeg stream (one seek,
//! then frame after frame), nearest to the cursor first; a newer request stops the running one.
//! During playback and while the timeline is dragged nothing new is decoded.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

use eframe::egui;
use microcut_engine::BufferWindow;
use microcut_media::{MediaBackend, MediaInfo, VideoFrame};

use crate::config::FrameBufferConfig;

/// What the worker should decode.
struct Job {
    id: u64,
    epoch: u64,
    media: Arc<MediaInfo>,
    size: (u32, u32),
    /// `(first frame, count)`, in order.
    runs: Vec<(i64, i64)>,
}

/// A decoded frame: epoch, frame index, picture.
type Loaded = (u64, i64, VideoFrame);

#[derive(Default)]
struct Shared {
    job: Mutex<Option<Job>>,
    wake: Condvar,
    /// The newest job; the worker stops a job as soon as it is no longer this one.
    current: AtomicU64,
    shutdown: AtomicBool,
}

/// The frame buffer of the open video. One per app; [`FrameBuffer::reset`] when another file
/// is opened.
pub struct FrameBuffer {
    config: FrameBufferConfig,
    shared: Arc<Shared>,
    results: Receiver<Loaded>,
    worker_alive: bool,
    /// Bumped when the file or the picture size changes; older results are dropped.
    epoch: u64,
    next_job: u64,
    size: (u32, u32),
    window: Option<BufferWindow>,
    frames: HashMap<i64, VideoFrame>,
    /// A job was stopped before it finished (playback, dragging), so frames may be missing
    /// although the window did not move.
    interrupted: bool,
}

impl FrameBuffer {
    /// Starts the worker.
    pub fn new(
        backend: Arc<dyn MediaBackend>,
        ctx: egui::Context,
        config: FrameBufferConfig,
    ) -> FrameBuffer {
        let shared = Arc::new(Shared::default());
        let (tx, results) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let worker_alive = thread::Builder::new()
            .name("microcut-framebuffer".to_owned())
            .spawn(move || worker(&worker_shared, backend.as_ref(), &tx, &ctx))
            .is_ok();
        FrameBuffer {
            config,
            shared,
            results,
            worker_alive,
            epoch: 0,
            next_job: 0,
            size: (0, 0),
            window: None,
            frames: HashMap::new(),
            interrupted: false,
        }
    }

    /// Forgets all frames (a different file was opened).
    pub fn reset(&mut self) {
        self.stop();
        self.epoch = self.epoch.wrapping_add(1);
        self.window = None;
        self.frames.clear();
        self.interrupted = false;
    }

    /// The decoded frame `frame`, if the buffer holds it.
    pub fn get(&self, frame: i64) -> Option<&VideoFrame> {
        self.frames.get(&frame)
    }

    /// Takes decoded frames and, if `active` (paused, not dragging), moves the window to
    /// `cursor` when needed and asks the worker for what is missing. `size` is the preview's
    /// size in physical pixels: frames are decoded to fit it.
    pub fn update(
        &mut self,
        media: &Arc<MediaInfo>,
        (cursor, last_frame): (i64, i64),
        size: (u32, u32),
        active: bool,
    ) {
        self.receive();
        if !self.worker_alive || size.0 == 0 || size.1 == 0 {
            return;
        }
        if size != self.size {
            self.reset();
            self.size = size;
        }
        if !active {
            if !self.interrupted {
                self.stop();
                self.interrupted = true;
            }
            return;
        }
        let radius = radius(self.config, media, size);
        if radius == 0 {
            self.frames.clear();
            self.window = None;
            return;
        }
        let window = match self.window {
            Some(window) => window.follow(cursor, radius, last_frame),
            None => BufferWindow::around(cursor, radius, last_frame),
        };
        if Some(window) == self.window && !self.interrupted {
            return;
        }
        self.window = Some(window);
        self.interrupted = false;
        self.frames.retain(|frame, _| window.contains(*frame));
        let runs = window.missing_runs(cursor, |frame| self.frames.contains_key(&frame));
        if !runs.is_empty() {
            self.request(Arc::clone(media), runs);
        }
    }

    fn request(&mut self, media: Arc<MediaInfo>, runs: Vec<(i64, i64)>) {
        self.next_job = self.next_job.wrapping_add(1);
        let job = Job {
            id: self.next_job,
            epoch: self.epoch,
            media,
            size: self.size,
            runs,
        };
        self.shared.current.store(job.id, Ordering::Release);
        *self
            .shared
            .job
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(job);
        self.shared.wake.notify_one();
    }

    /// Stops the running job and drops a waiting one.
    fn stop(&mut self) {
        self.next_job = self.next_job.wrapping_add(1);
        self.shared.current.store(self.next_job, Ordering::Release);
        *self
            .shared
            .job
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn receive(&mut self) {
        while let Ok((epoch, frame, picture)) = self.results.try_recv() {
            let wanted = self.window.is_some_and(|w| w.contains(frame));
            if epoch == self.epoch && wanted {
                self.frames.insert(frame, picture);
            }
        }
    }
}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        self.stop();
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.wake.notify_all();
    }
}

/// Frames to keep to either side: as configured, but fewer if the pictures would take more
/// memory than allowed (estimated from the largest picture that fits `size`).
fn radius(config: FrameBufferConfig, media: &MediaInfo, size: (u32, u32)) -> i64 {
    let (w, h) = media.video.display_size();
    let bytes = u64::from(w.min(size.0).max(1))
        .saturating_mul(u64::from(h.min(size.1).max(1)))
        .saturating_mul(4);
    let budget = config.max_megabytes.saturating_mul(1_000_000);
    // The window holds 2 · radius + 1 frames.
    let fits = (budget / bytes.max(1)).saturating_sub(1) / 2;
    config
        .frames_each_way
        .clamp(0, i64::try_from(fits).unwrap_or(i64::MAX))
}

fn worker(
    shared: &Shared,
    backend: &dyn MediaBackend,
    results: &Sender<Loaded>,
    ctx: &egui::Context,
) {
    loop {
        let job = {
            let mut slot = shared.job.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = slot.take() {
                    break job;
                }
                slot = shared
                    .wake
                    .wait(slot)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        // A failed decode only leaves frames out; the preview decodes those itself.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            run_job(shared, backend, results, ctx, &job)
        }));
        if matches!(outcome, Ok(Err(Gone))) {
            return;
        }
    }
}

/// The buffer was dropped.
struct Gone;

fn run_job(
    shared: &Shared,
    backend: &dyn MediaBackend,
    results: &Sender<Loaded>,
    ctx: &egui::Context,
    job: &Job,
) -> Result<(), Gone> {
    let still_wanted = || {
        shared.current.load(Ordering::Acquire) == job.id && !shared.shutdown.load(Ordering::Acquire)
    };
    for &(first, count) in &job.runs {
        if !still_wanted() {
            return Ok(());
        }
        let Ok(mut stream) = backend.video_stream(&job.media, first, count, job.size) else {
            continue;
        };
        let mut frame = first;
        // Dropping the stream stops ffmpeg.
        while still_wanted() && frame < first.saturating_add(count) {
            let Ok(Some(picture)) = stream.next_frame() else {
                break;
            };
            results
                .send((job.epoch, frame, picture))
                .map_err(|_| Gone)?;
            ctx.request_repaint();
            frame = frame.saturating_add(1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use microcut_time::{FrameRate, Tick};

    fn media(width: u32, height: u32) -> MediaInfo {
        MediaInfo {
            path: "clip.mp4".into(),
            container: "mp4".to_owned(),
            start: Tick::ZERO,
            duration: Tick::from_secs(60),
            video: microcut_media::VideoInfo {
                codec: "h264".to_owned(),
                width,
                height,
                frame_rate: FrameRate::new(60, 1).unwrap(),
                rotation: 0,
            },
            audio: Vec::new(),
        }
    }

    #[test]
    fn the_radius_respects_the_memory_limit() {
        let config = FrameBufferConfig::default(); // 32 each way, 1024 MB
        // 720p in a large preview: 3.7 MB a frame, 65 frames fit easily.
        assert_eq!(radius(config, &media(1280, 720), (1920, 1080)), 32);
        // 4K in a 4K preview: 33 MB a frame, 1024 MB hold 30 frames → 14 each way.
        assert_eq!(radius(config, &media(3840, 2160), (3840, 2160)), 14);
        // Turned off, or no memory at all.
        let off = FrameBufferConfig {
            frames_each_way: 0,
            ..config
        };
        assert_eq!(radius(off, &media(1280, 720), (1280, 720)), 0);
        let none = FrameBufferConfig {
            max_megabytes: 0,
            ..config
        };
        assert_eq!(radius(none, &media(1280, 720), (1280, 720)), 0);
        // Odd sizes stay finite.
        assert_eq!(radius(config, &media(0, 0), (0, 0)), 32);
    }

    /// Feeds the buffer like the app does until it holds `frames` (or gives up after 30 s).
    fn fill(
        buffer: &mut FrameBuffer,
        media: &Arc<MediaInfo>,
        cursor: i64,
        size: (u32, u32),
        frames: std::ops::RangeInclusive<i64>,
    ) {
        let last = media.frame_count() - 1;
        let started = std::time::Instant::now();
        loop {
            buffer.update(media, (cursor, last), size, true);
            if frames.clone().all(|f| buffer.get(f).is_some()) {
                return;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(30),
                "buffer did not fill {frames:?}"
            );
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// With real ffmpeg: the buffer holds the frames around the cursor, each the same picture
    /// as decoding that frame on its own, and moving the window keeps the shared frames.
    #[test]
    fn buffered_frames_are_the_frames_around_the_cursor() {
        let Ok(backend) = microcut_media::FfmpegBackend::locate() else {
            eprintln!("skipping: FFmpeg not found");
            return;
        };
        let dir = std::env::temp_dir().join(format!("microcut-framebuffer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("count_25fps.mp4");
        // A moving test picture: neighbouring frames differ clearly.
        let status = std::process::Command::new(backend.ffmpeg_path())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=160x96:r=25:d=4",
            ])
            .args(["-c:v", "mpeg4", "-q:v", "2", "-g", "50"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success());
        let media = Arc::new(backend.probe(&clip).unwrap());
        let backend: Arc<dyn MediaBackend> = Arc::new(backend);
        let config = FrameBufferConfig {
            frames_each_way: 8,
            max_megabytes: 1024,
        };
        let size = (160, 96);
        let mut buffer = FrameBuffer::new(Arc::clone(&backend), egui::Context::default(), config);

        fill(&mut buffer, &media, 40, size, 32..=48);
        assert!(buffer.get(31).is_none() && buffer.get(49).is_none());
        for frame in [32, 40, 48] {
            let alone = backend.decode_frame(&media, frame, size).unwrap();
            assert!(buffer.get(frame) == Some(&alone), "frame {frame} differs");
        }
        // Neighbours differ, so equality above really pins the frame number.
        assert!(buffer.get(40) != buffer.get(41));

        // Four frames on: still more than half the radius (4) ahead → the window stays.
        fill(&mut buffer, &media, 44, size, 32..=48);
        assert_eq!(buffer.window.map(|w| (w.first(), w.last())), Some((32, 48)));
        // Past that: the window moves to 46 (38 ..= 54); 38 ..= 48 are kept, 49 ..= 54 added.
        let kept = buffer.get(45).cloned();
        fill(&mut buffer, &media, 46, size, 38..=54);
        assert_eq!(buffer.window.map(|w| (w.first(), w.last())), Some((38, 54)));
        assert!(
            buffer.get(37).is_none(),
            "frames outside the window are dropped"
        );
        assert_eq!(buffer.get(45).cloned(), kept);
        let alone = backend.decode_frame(&media, 54, size).unwrap();
        assert!(buffer.get(54) == Some(&alone), "frame 54 differs");

        drop(buffer);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
