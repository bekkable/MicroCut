//! Thumbnails for the video strip, decoded by a small pool of worker threads.
//!
//! Every UI frame the timeline says which frames it shows ([`Thumbnails::want`]). Missing ones
//! are queued; the queue is replaced each time, so after zooming or scrolling only frames that
//! are still visible get decoded. Decoded thumbnails are kept (least recently used dropped).

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

use eframe::egui;
use microcut_media::{MediaBackend, MediaInfo, VideoFrame};

use crate::preview::color_image;

/// Decoder threads (each runs one ffmpeg at a time).
const WORKERS: usize = 2;

/// Thumbnails kept in memory.
const CAPACITY: usize = 240;

struct Job {
    generation: u64,
    frame: i64,
    size: (u32, u32),
    media: Arc<MediaInfo>,
}

type Done = (u64, i64, Result<VideoFrame, String>);

#[derive(Default)]
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    wake: Condvar,
    shutdown: AtomicBool,
}

struct Entry {
    texture: egui::TextureHandle,
    used: u64,
}

/// Thumbnail cache of the open video. One per app; [`Thumbnails::reset`] when another file is
/// opened.
pub struct Thumbnails {
    queue: Arc<Queue>,
    results: Receiver<Done>,
    workers_alive: bool,
    /// Bumped when the file or the thumbnail size changes; older results are ignored.
    generation: u64,
    size: (u32, u32),
    textures: HashMap<i64, Entry>,
    /// Queued or being decoded.
    outstanding: HashSet<i64>,
    failed: HashSet<i64>,
    clock: u64,
}

impl Thumbnails {
    /// Starts the decode workers.
    pub fn new(backend: &Arc<dyn MediaBackend>, ctx: &egui::Context) -> Thumbnails {
        let queue = Arc::new(Queue::default());
        let (tx, results) = mpsc::channel();
        let mut workers_alive = false;
        for n in 0..WORKERS {
            let (queue, backend, tx, ctx) = (
                Arc::clone(&queue),
                Arc::clone(backend),
                tx.clone(),
                ctx.clone(),
            );
            workers_alive |= thread::Builder::new()
                .name(format!("microcut-thumbs-{n}"))
                .spawn(move || worker(&queue, backend.as_ref(), &tx, &ctx))
                .is_ok();
        }
        Thumbnails {
            queue,
            results,
            workers_alive,
            generation: 0,
            size: (0, 0),
            textures: HashMap::new(),
            outstanding: HashSet::new(),
            failed: HashSet::new(),
            clock: 0,
        }
    }

    /// Forgets all thumbnails (a different file was opened).
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.textures.clear();
        self.outstanding.clear();
        self.failed.clear();
        self.lock_jobs().clear();
    }

    /// Takes finished thumbnails and queues the missing ones of `frames` (in that order), decoded
    /// to fit `size` physical pixels.
    pub fn want(
        &mut self,
        ctx: &egui::Context,
        media: &Arc<MediaInfo>,
        frames: &[i64],
        size: (u32, u32),
    ) {
        if size != self.size {
            self.reset();
            self.size = size;
        }
        self.receive(ctx);
        self.clock = self.clock.wrapping_add(1);
        for frame in frames {
            if let Some(entry) = self.textures.get_mut(frame) {
                entry.used = self.clock;
            }
        }
        if !self.workers_alive || size.0 == 0 || size.1 == 0 {
            return;
        }

        let mut jobs = self
            .queue
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Jobs nobody has started yet are replaced by the current wish list.
        for job in jobs.drain(..) {
            self.outstanding.remove(&job.frame);
        }
        for &frame in frames {
            let known = self.textures.contains_key(&frame)
                || self.failed.contains(&frame)
                || self.outstanding.contains(&frame);
            if !known {
                self.outstanding.insert(frame);
                jobs.push_back(Job {
                    generation: self.generation,
                    frame,
                    size,
                    media: Arc::clone(media),
                });
            }
        }
        if !jobs.is_empty() {
            self.queue.wake.notify_all();
        }
        drop(jobs);
        self.evict();
    }

    /// The thumbnail of `frame`, if decoded.
    pub fn get(&self, frame: i64) -> Option<&egui::TextureHandle> {
        self.textures.get(&frame).map(|entry| &entry.texture)
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((generation, frame, result)) = self.results.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.outstanding.remove(&frame);
            let image = result.ok().and_then(|decoded| color_image(&decoded));
            match image {
                Some(image) => {
                    let texture = ctx.load_texture(
                        format!("thumb-{frame}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(
                        frame,
                        Entry {
                            texture,
                            used: self.clock,
                        },
                    );
                }
                // The strip shows a placeholder; the preview reports decode errors.
                None => {
                    self.failed.insert(frame);
                }
            }
        }
    }

    /// Drops the least recently used thumbnails beyond [`CAPACITY`].
    fn evict(&mut self) {
        let excess = self.textures.len().saturating_sub(CAPACITY);
        if excess == 0 {
            return;
        }
        let mut by_age: Vec<(u64, i64)> = self
            .textures
            .iter()
            .map(|(&frame, entry)| (entry.used, frame))
            .collect();
        by_age.sort_unstable();
        for (_, frame) in by_age.into_iter().take(excess) {
            self.textures.remove(&frame);
        }
    }

    fn lock_jobs(&self) -> std::sync::MutexGuard<'_, VecDeque<Job>> {
        self.queue
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        self.queue.shutdown.store(true, Ordering::Release);
        self.queue.wake.notify_all();
    }
}

fn worker(queue: &Queue, backend: &dyn MediaBackend, results: &Sender<Done>, ctx: &egui::Context) {
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if queue.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                jobs = queue
                    .wake
                    .wait(jobs)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let decoded = panic::catch_unwind(AssertUnwindSafe(|| {
            backend.decode_frame(&job.media, job.frame, job.size)
        }));
        let result = match decoded {
            Ok(frame) => frame.map_err(|err| err.to_string()),
            Err(_) => Err("internal error while decoding a thumbnail".to_owned()),
        };
        if results.send((job.generation, job.frame, result)).is_err() {
            return; // The cache is gone.
        }
        ctx.request_repaint();
    }
}
