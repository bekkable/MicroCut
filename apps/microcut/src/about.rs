//! The "About MicroCut" dialog: version, purpose, license, the FFmpeg in use and a link to the
//! project. FFmpeg's version is read on a worker thread (it may run `ffmpeg -version`).

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui;
use microcut_media::FfmpegBackend;

use crate::ICON_PNG;

/// Where the project lives (from `Cargo.toml`).
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Size of the icon in the dialog (points).
const ICON_SIZE: f32 = 72.0;

/// What the dialog says about FFmpeg.
enum Ffmpeg {
    /// Not found: why.
    Missing(String),
    Found {
        path: String,
        /// The version once known: `Some(None)` for a development build without a number.
        version: Option<Option<String>>,
        pending: Option<Receiver<Option<String>>>,
    },
}

/// The open dialog.
pub struct AboutDialog {
    icon: Option<egui::TextureHandle>,
    ffmpeg: Ffmpeg,
    /// Why the project page could not be opened.
    note: Option<String>,
    /// Set after the first frame: the Enter that opened the dialog must not also close it.
    armed: bool,
}

impl AboutDialog {
    /// A dialog describing `ffmpeg` (or why it is missing).
    pub fn new(ctx: &egui::Context, ffmpeg: Result<&Arc<FfmpegBackend>, &str>) -> AboutDialog {
        let ffmpeg = match ffmpeg {
            Err(why) => Ffmpeg::Missing(why.to_owned()),
            Ok(backend) => Ffmpeg::Found {
                path: backend.ffmpeg_path().display().to_string(),
                version: None,
                pending: version_job(Arc::clone(backend), ctx.clone()),
            },
        };
        AboutDialog {
            icon: icon_texture(ctx),
            ffmpeg,
            note: None,
            armed: false,
        }
    }

    /// Shows the dialog. Returns `false` once it is closed (Close, Escape, Enter or a click
    /// outside).
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        self.poll();
        let modal = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(500.0);
            ui.horizontal(|ui| {
                if let Some(icon) = &self.icon {
                    ui.add(egui::Image::new((
                        icon.id(),
                        egui::vec2(ICON_SIZE, ICON_SIZE),
                    )));
                    ui.add_space(12.0);
                }
                ui.vertical(|ui| {
                    ui.heading("MicroCut");
                    ui.label(format!("Version {VERSION}"));
                    ui.add_space(4.0);
                    ui.label("A minimal video trimmer: pick a start and an end, export the cut.");
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            self.facts(ui);
            ui.add_space(10.0);
            if let Some(note) = &self.note {
                ui.colored_label(ui.visuals().error_fg_color, note);
                ui.add_space(6.0);
            }
            let armed = self.armed;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.button("Close");
                close.clicked() || (armed && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            })
            .inner
        });
        self.armed = true;
        let closed = modal.inner || modal.should_close();
        !closed
    }

    /// License, project page and FFmpeg.
    fn facts(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("about facts")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.weak("License");
                ui.label("MIT OR Apache-2.0, at your option");
                ui.end_row();

                ui.weak("Project");
                let link = ui
                    .link(REPOSITORY)
                    .on_hover_text("Open the project page in the browser");
                if link.clicked() {
                    self.note = open_url(REPOSITORY).err();
                }
                ui.end_row();

                ui.weak("FFmpeg");
                ui.vertical(|ui| match &self.ffmpeg {
                    Ffmpeg::Missing(why) => {
                        ui.colored_label(ui.visuals().error_fg_color, why);
                    }
                    Ffmpeg::Found { path, version, .. } => {
                        let version = match version {
                            None => "…".to_owned(),
                            Some(Some(v)) => v.clone(),
                            Some(None) => "development build".to_owned(),
                        };
                        ui.label(format!("FFmpeg {version}"));
                        ui.add(egui::Label::new(egui::RichText::new(path).weak()).truncate());
                    }
                });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.weak(
            "MicroCut runs FFmpeg as a separate program. FFmpeg is not part of MicroCut and keeps \
             its own license (LGPL/GPL).",
        );
    }

    /// Takes the FFmpeg version once the worker has it.
    fn poll(&mut self) {
        let Ffmpeg::Found {
            version, pending, ..
        } = &mut self.ffmpeg
        else {
            return;
        };
        let Some(rx) = pending else {
            return;
        };
        match rx.try_recv() {
            Ok(found) => {
                *version = Some(found);
                *pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                *version = Some(Some("unknown".to_owned()));
                *pending = None;
            }
        }
    }
}

/// Reads FFmpeg's version on a worker thread (cached by the backend after the first time).
fn version_job(
    backend: Arc<FfmpegBackend>,
    ctx: egui::Context,
) -> Option<Receiver<Option<String>>> {
    let (tx, rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("microcut-about".to_owned())
        .spawn(move || {
            let version = panic::catch_unwind(AssertUnwindSafe(|| backend.version()))
                .ok()
                .flatten()
                .map(|v| v.to_string());
            let _ = tx.send(version);
            ctx.request_repaint();
        });
    spawned.ok().map(|_| rx)
}

/// The app icon as a texture (mipmapped, so it stays smooth at dialog size).
fn icon_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG).ok()?;
    let (w, h) = (
        usize::try_from(icon.width).ok()?,
        usize::try_from(icon.height).ok()?,
    );
    if w.checked_mul(h)?.checked_mul(4)? != icon.rgba.len() {
        return None;
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &icon.rgba);
    let options = egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
    Some(ctx.load_texture("about icon", image, options))
}

/// Opens `url` in the default browser (through the system, like "Show in folder").
fn open_url(url: &str) -> Result<(), String> {
    open_url_command(url)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Could not open the browser: {err}"))
}

#[cfg(windows)]
fn open_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("explorer");
    command.arg(url);
    command
}

#[cfg(target_os = "macos")]
fn open_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(url);
    command
}

#[cfg(not(any(windows, target_os = "macos")))]
fn open_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_page_is_the_repository() {
        assert!(
            REPOSITORY.starts_with("https://github.com/"),
            "{REPOSITORY}"
        );
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn the_icon_becomes_a_texture() {
        let ctx = egui::Context::default();
        let texture = icon_texture(&ctx).unwrap();
        assert_eq!(texture.size(), [1024, 1024]);
    }
}
