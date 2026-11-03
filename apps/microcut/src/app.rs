//! Top-level UI state and frame loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use microcut_media::{FfmpegBackend, MediaBackend, MediaInfo, clamp_frame};
use microcut_time::{ClockStamp, FrameRate};

use crate::loader::ProbeJob;
use crate::preview::Preview;

/// File extensions offered in the open dialog. Anything ffmpeg reads can still be dropped.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mpg", "mpeg", "wmv", "flv",
    "3gp", "ogv", "mxf",
];

const OPEN_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);

/// An opened video and the cursor in it.
struct Loaded {
    info: Arc<MediaInfo>,
    /// Frame index at the cursor; always within the video.
    cursor: i64,
}

impl Loaded {
    fn last_frame(&self) -> i64 {
        clamp_frame(&self.info, i64::MAX)
    }

    fn move_cursor(&mut self, to: i64) {
        self.cursor = clamp_frame(&self.info, to);
    }
}

/// What is open right now.
enum Document {
    Empty,
    Probing(ProbeJob),
    Loaded(Loaded),
    Failed { path: PathBuf, message: String },
}

/// Root application state.
pub struct MicroCutApp {
    /// The media backend, or why it is unavailable (shown permanently).
    backend: Result<Arc<dyn MediaBackend>, String>,
    preview: Option<Preview>,
    document: Document,
    ctx: egui::Context,
}

impl MicroCutApp {
    /// Creates the app and starts opening `initial` (from the command line), if given.
    pub fn new(ctx: &egui::Context, initial: Option<PathBuf>) -> MicroCutApp {
        let backend = FfmpegBackend::locate()
            .map(|b| Arc::new(b) as Arc<dyn MediaBackend>)
            .map_err(|err| err.to_string());
        let preview = backend
            .as_ref()
            .ok()
            .map(|b| Preview::new(Arc::clone(b), ctx.clone()));
        let mut app = MicroCutApp {
            backend,
            preview,
            document: Document::Empty,
            ctx: ctx.clone(),
        };
        if let Some(path) = initial {
            app.open(path);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        self.document = match &self.backend {
            Ok(backend) => {
                Document::Probing(ProbeJob::start(Arc::clone(backend), path, self.ctx.clone()))
            }
            // The full reason is already shown in the toolbar.
            Err(_) => Document::Failed {
                path,
                message: "FFmpeg is needed to open videos (see above).".to_owned(),
            },
        };
    }

    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open video")
            .add_filter("Video", VIDEO_EXTENSIONS)
            .add_filter("All files", &["*"]);
        if let Some(dir) = self.current_path().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(path);
        }
    }

    fn current_path(&self) -> Option<&Path> {
        match &self.document {
            Document::Empty => None,
            Document::Probing(job) => Some(job.path()),
            Document::Loaded(loaded) => Some(&loaded.info.path),
            Document::Failed { path, .. } => Some(path),
        }
    }

    fn poll_probe(&mut self) {
        let Document::Probing(job) = &self.document else {
            return;
        };
        let Some(result) = job.poll() else {
            return;
        };
        let (title, document) = match result {
            Ok(info) => {
                if let Some(preview) = &mut self.preview {
                    preview.reset();
                }
                (
                    format!("MicroCut — {}", info.file_name()),
                    Document::Loaded(Loaded {
                        info: Arc::new(info),
                        cursor: 0,
                    }),
                )
            }
            Err(message) => (
                "MicroCut".to_owned(),
                Document::Failed {
                    path: job.path().clone(),
                    message,
                },
            ),
        };
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Title(title));
        self.document = document;
    }

    fn handle_input(&mut self, ui: &mut egui::Ui) {
        let dropped = ui.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(path);
        }
        if ui.input_mut(|i| i.consume_shortcut(&OPEN_SHORTCUT)) {
            self.open_dialog();
        }
        if let Document::Loaded(loaded) = &mut self.document {
            cursor_keys(ui, loaded);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let open = ui
                .button("Open…")
                .on_hover_text(ui.ctx().format_shortcut(&OPEN_SHORTCUT));
            if open.clicked() {
                self.open_dialog();
            }
            if let Some(path) = self.current_path() {
                ui.label(path.display().to_string());
            }
        });
        if let Err(message) = &self.backend {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
    }

    fn document_view(&mut self, ui: &mut egui::Ui) {
        match &mut self.document {
            Document::Empty => {
                ui.centered_and_justified(|ui| {
                    ui.label("Drop a video here or click Open…");
                });
            }
            Document::Probing(job) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading {}…", display_name(job.path())));
                });
            }
            Document::Loaded(loaded) => {
                if let Some(preview) = &mut self.preview {
                    preview.show(ui, &loaded.info, loaded.cursor);
                }
            }
            Document::Failed { path, message } => {
                ui.heading(format!("Could not open {}", display_name(path)));
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
        }
    }
}

impl eframe::App for MicroCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_input(ui);
        self.poll_probe();

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if let Document::Loaded(loaded) = &mut self.document {
            let preview = self.preview.as_ref();
            egui::Panel::bottom("transport").show(ui, |ui| transport(ui, loaded, preview));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0.0))
            .show(ui, |ui| {
                self.document_view(ui);
                drop_hint(ui);
            });
    }
}

/// Temporary cursor control until the timeline (M3): a frame slider and a status line.
fn transport(ui: &mut egui::Ui, loaded: &mut Loaded, preview: Option<&Preview>) {
    ui.add_space(4.0);
    let last = loaded.last_frame();
    ui.spacing_mut().slider_width = ui.available_width();
    let mut cursor = loaded.cursor;
    let slider = ui.add(egui::Slider::new(&mut cursor, 0..=last).show_value(false));
    loaded.move_cursor(cursor);
    // Keep keyboard focus free, so the arrow keys always step frames the same way.
    if slider.has_focus() {
        slider.surrender_focus();
    }

    ui.horizontal(|ui| {
        let rate = loaded.info.video.frame_rate;
        let time = ClockStamp(rate.frame_to_tick(loaded.cursor));
        ui.monospace(format!("{time}   frame {} / {last}", loaded.cursor));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(media_summary(&loaded.info));
            if let Some(preview) = preview {
                if let Some(error) = preview.error() {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                } else if preview.is_busy() {
                    ui.spinner();
                }
            }
        });
    });
    ui.add_space(2.0);
}

/// ←/→ one frame, Shift+←/→ one second, Home/End first/last frame. Only when no text field or
/// other widget has the keyboard.
fn cursor_keys(ui: &mut egui::Ui, loaded: &mut Loaded) {
    if ui.memory(|m| m.focused().is_some()) {
        return;
    }
    let second = frames_per_second(loaded.info.video.frame_rate);
    let last = loaded.last_frame();
    let target = ui.input_mut(|i| {
        let shift = egui::Modifiers::SHIFT;
        let none = egui::Modifiers::NONE;
        // Shift variants first: `consume_key(NONE, …)` would also match Shift+key.
        if i.consume_key(shift, egui::Key::ArrowLeft) {
            Some(loaded.cursor.saturating_sub(second))
        } else if i.consume_key(shift, egui::Key::ArrowRight) {
            Some(loaded.cursor.saturating_add(second))
        } else if i.consume_key(none, egui::Key::ArrowLeft) {
            Some(loaded.cursor.saturating_sub(1))
        } else if i.consume_key(none, egui::Key::ArrowRight) {
            Some(loaded.cursor.saturating_add(1))
        } else if i.consume_key(none, egui::Key::Home) {
            Some(0)
        } else if i.consume_key(none, egui::Key::End) {
            Some(last)
        } else {
            None
        }
    });
    if let Some(target) = target {
        loaded.move_cursor(target);
    }
}

/// Frames in one second, rounded (at least 1).
fn frames_per_second(rate: FrameRate) -> i64 {
    let (num, den) = (i64::from(rate.num()), i64::from(rate.den()));
    ((num + den / 2) / den).max(1)
}

/// `h264 1280×720 · 60 fps · aac 48 kHz stereo · 00:08:55.300`
fn media_summary(info: &MediaInfo) -> String {
    let video = &info.video;
    let (w, h) = video.display_size();
    let mut parts = vec![
        format!("{} {w}×{h}", video.codec),
        format_fps(video.frame_rate),
    ];
    if let Some(audio) = &info.audio {
        let mut text = audio.codec.clone();
        if let Some(rate) = audio.sample_rate {
            text.push_str(&format!(" {} kHz", f64::from(rate) / 1000.0));
        }
        match audio.channels {
            Some(1) => text.push_str(" mono"),
            Some(2) => text.push_str(" stereo"),
            Some(n) => text.push_str(&format!(" {n} ch")),
            None => {}
        }
        parts.push(text);
    }
    parts.push(ClockStamp(info.duration).to_string());
    parts.join(" · ")
}

/// Highlights the window while a file is dragged over it.
fn drop_hint(ui: &mut egui::Ui) {
    if ui.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(160));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to open",
        egui::FontId::proportional(24.0),
        egui::Color32::WHITE,
    );
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// `25 fps`, `29.97 fps`, `23.976 fps` (display only; edit math stays rational).
fn format_fps(rate: FrameRate) -> String {
    if rate.den() == 1 {
        return format!("{} fps", rate.num());
    }
    let fps = f64::from(rate.num()) / f64::from(rate.den());
    let text = format!("{fps:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} fps")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_is_shown_compactly() {
        let fps = |n, d| format_fps(FrameRate::new(n, d).unwrap());
        assert_eq!(fps(25, 1), "25 fps");
        assert_eq!(fps(30_000, 1_001), "29.97 fps");
        assert_eq!(fps(24_000, 1_001), "23.976 fps");
        assert_eq!(fps(50, 2), "25 fps");
    }

    #[test]
    fn one_second_in_frames() {
        let fps = |n, d| frames_per_second(FrameRate::new(n, d).unwrap());
        assert_eq!(fps(60, 1), 60);
        assert_eq!(fps(30_000, 1_001), 30);
        assert_eq!(fps(1, 10), 1);
    }
}
