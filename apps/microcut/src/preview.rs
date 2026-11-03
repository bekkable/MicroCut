//! The preview pane: shows the frame at the cursor, decoded on a worker thread.
//!
//! The worker serves a one-slot mailbox: a new request replaces a pending one, so while
//! scrubbing only the newest position is decoded ("latest request wins").

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;

use eframe::egui;
use microcut_media::{MediaBackend, MediaInfo, VideoFrame};

/// Identifies a picture: which document, which frame, which pixel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    generation: u64,
    frame: i64,
    size: (u32, u32),
}

struct Job {
    key: Key,
    media: Arc<MediaInfo>,
}

type Response = (Key, Result<VideoFrame, String>);

/// State shared with the worker.
#[derive(Default)]
struct Mailbox {
    job: Mutex<Option<Job>>,
    wake: Condvar,
    shutdown: AtomicBool,
}

/// Preview pane state. One per app; [`Preview::reset`] when another file is opened.
pub struct Preview {
    mailbox: Arc<Mailbox>,
    results: Receiver<Response>,
    worker_alive: bool,
    generation: u64,
    requested: Option<Key>,
    shown: Option<Key>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
    /// Size of the preview area in physical pixels at the last repaint.
    size: (u32, u32),
}

impl Preview {
    /// Starts the decode worker.
    pub fn new(backend: Arc<dyn MediaBackend>, ctx: egui::Context) -> Preview {
        let mailbox = Arc::new(Mailbox::default());
        let (tx, results) = mpsc::channel();
        let worker_mailbox = Arc::clone(&mailbox);
        let worker_alive = thread::Builder::new()
            .name("microcut-preview".to_owned())
            .spawn(move || worker(&worker_mailbox, backend.as_ref(), &tx, &ctx))
            .is_ok();
        Preview {
            mailbox,
            results,
            worker_alive,
            generation: 0,
            requested: None,
            shown: None,
            texture: None,
            error: (!worker_alive).then(|| "could not start the preview decoder".to_owned()),
            size: (0, 0),
        }
    }

    /// Size of the preview area in physical pixels (zero before the first repaint).
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Shows `picture`, decoded elsewhere (playback), as frame `frame`. Decodes still running
    /// for earlier requests are ignored, and none is started for this frame.
    pub fn present(&mut self, ctx: &egui::Context, frame: i64, picture: &VideoFrame) {
        self.generation = self.generation.wrapping_add(1);
        self.upload(ctx, picture);
        let key = Key {
            generation: self.generation,
            frame,
            size: self.size,
        };
        self.requested = Some(key);
        self.shown = Some(key);
        self.error = None;
    }

    /// Forgets the current picture (a different file was opened).
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.requested = None;
        self.shown = None;
        self.texture = None;
        self.error = None;
    }

    /// Whether a requested picture has not arrived yet.
    pub fn is_busy(&self) -> bool {
        self.worker_alive && self.requested.is_some() && self.requested != self.shown
    }

    /// Last decode error, if the newest result was one.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Fills the available space with frame `frame` of `media`. Returns whether the picture was
    /// clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, media: &Arc<MediaInfo>, frame: i64) -> bool {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
        let pixels_per_point = ui.ctx().pixels_per_point();
        let size = (
            physical_pixels(rect.width(), pixels_per_point),
            physical_pixels(rect.height(), pixels_per_point),
        );
        self.size = size;

        let key = Key {
            generation: self.generation,
            frame,
            size,
        };
        if self.requested != Some(key) && size.0 > 0 && size.1 > 0 {
            self.request(key, media);
        }
        self.receive(ui.ctx());

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        if let Some(texture) = &self.texture {
            let fitted = fit_rect(texture.size_vec2(), rect);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), fitted, uv, egui::Color32::WHITE);
        }
        response.clicked()
    }

    fn request(&mut self, key: Key, media: &Arc<MediaInfo>) {
        self.requested = Some(key);
        let job = Job {
            key,
            media: Arc::clone(media),
        };
        *self
            .mailbox
            .job
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(job);
        self.mailbox.wake.notify_one();
    }

    fn receive(&mut self, ctx: &egui::Context) {
        // Drain everything; only the newest result of the current document matters.
        while let Ok((key, result)) = self.results.try_recv() {
            if key.generation != self.generation {
                continue;
            }
            match result {
                Ok(frame) => {
                    self.upload(ctx, &frame);
                    self.error = None;
                }
                // Keep the previous picture; say why the new one is missing.
                Err(message) => self.error = Some(message),
            }
            self.shown = Some(key);
        }
    }

    fn upload(&mut self, ctx: &egui::Context, frame: &VideoFrame) {
        let Some(image) = color_image(frame) else {
            return;
        };
        let options = egui::TextureOptions::LINEAR;
        match &mut self.texture {
            Some(texture) => texture.set(image, options),
            None => self.texture = Some(ctx.load_texture("preview", image, options)),
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.mailbox.shutdown.store(true, Ordering::Release);
        self.mailbox.wake.notify_all();
    }
}

fn worker(
    mailbox: &Mailbox,
    backend: &dyn MediaBackend,
    results: &Sender<Response>,
    ctx: &egui::Context,
) {
    loop {
        let job = {
            let mut slot = mailbox.job.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if mailbox.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = slot.take() {
                    break job;
                }
                slot = mailbox
                    .wake
                    .wait(slot)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let decoded = panic::catch_unwind(AssertUnwindSafe(|| {
            backend.decode_frame(&job.media, job.key.frame, job.key.size)
        }));
        let result = match decoded {
            Ok(frame) => frame.map_err(|err| err.to_string()),
            Err(_) => Err("internal error while decoding the frame".to_owned()),
        };
        if results.send((job.key, result)).is_err() {
            return; // The preview is gone.
        }
        ctx.request_repaint();
    }
}

/// A decoded frame as an egui image; `None` if the buffer does not match the size.
pub(crate) fn color_image(frame: &VideoFrame) -> Option<egui::ColorImage> {
    let w = usize::try_from(frame.width()).ok()?;
    let h = usize::try_from(frame.height()).ok()?;
    // `VideoFrame` guarantees this; checked again because `from_rgba_unmultiplied` asserts.
    if w.checked_mul(h).and_then(|p| p.checked_mul(4)) != Some(frame.rgba().len()) {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [w, h],
        frame.rgba(),
    ))
}

/// Points → whole physical pixels, clamped to `u32`.
pub(crate) fn physical_pixels(points: f32, pixels_per_point: f32) -> u32 {
    let px = (points * pixels_per_point).round();
    if px.is_finite() && px >= 1.0 {
        // In range after the clamp, so the cast is exact.
        px.min(u32::MAX as f32) as u32
    } else {
        0
    }
}

/// Largest rect with the aspect ratio of `size`, centred in `area` (may scale up).
fn fit_rect(size: egui::Vec2, area: egui::Rect) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    egui::Rect::from_center_size(area.center(), size * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_pixels_handles_odd_input() {
        assert_eq!(physical_pixels(100.0, 1.5), 150);
        assert_eq!(physical_pixels(0.2, 1.0), 0);
        assert_eq!(physical_pixels(-5.0, 1.0), 0);
        assert_eq!(physical_pixels(f32::NAN, 1.0), 0);
        assert_eq!(physical_pixels(f32::INFINITY, 1.0), 0);
        assert_eq!(physical_pixels(1e30, 1.0), u32::MAX);
    }

    #[test]
    fn fit_rect_letterboxes() {
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let fitted = fit_rect(egui::vec2(16.0, 9.0), area);
        assert!((fitted.height() - 100.0).abs() < 1e-3);
        assert!((fitted.width() - 177.777).abs() < 1e-2);
        assert!((fitted.center().x - 100.0).abs() < 1e-3);
        assert_eq!(fit_rect(egui::vec2(0.0, 5.0), area), area);
    }
}
