//! Top-level UI state and frame loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use microcut_media::{FfmpegBackend, MediaBackend, MediaInfo};
use microcut_time::{ClockStamp, FrameRate};

use crate::loader::ProbeJob;

/// File extensions offered in the open dialog. Anything ffmpeg reads can still be dropped.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mpg", "mpeg", "wmv", "flv",
    "3gp", "ogv", "mxf",
];

const OPEN_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);

/// What is open right now.
enum Document {
    Empty,
    Probing(ProbeJob),
    Loaded(MediaInfo),
    Failed { path: PathBuf, message: String },
}

/// Root application state.
pub struct MicroCutApp {
    /// The media backend, or why it is unavailable (shown permanently).
    backend: Result<Arc<dyn MediaBackend>, String>,
    document: Document,
    ctx: egui::Context,
}

impl MicroCutApp {
    /// Creates the app and starts opening `initial` (from the command line), if given.
    pub fn new(ctx: &egui::Context, initial: Option<PathBuf>) -> MicroCutApp {
        let backend = FfmpegBackend::locate()
            .map(|b| Arc::new(b) as Arc<dyn MediaBackend>)
            .map_err(|err| err.to_string());
        let mut app = MicroCutApp {
            backend,
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
            Document::Loaded(info) => Some(&info.path),
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
            Ok(info) => (
                format!("MicroCut — {}", info.file_name()),
                Document::Loaded(info),
            ),
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

    fn document_view(&self, ui: &mut egui::Ui) {
        match &self.document {
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
            Document::Loaded(info) => info_grid(ui, info),
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
        egui::CentralPanel::default().show(ui, |ui| {
            self.document_view(ui);
            drop_hint(ui);
        });
    }
}

fn info_grid(ui: &mut egui::Ui, info: &MediaInfo) {
    let video = &info.video;
    let mut video_text = format!(
        "{}, {} × {}, {}",
        video.codec,
        video.width,
        video.height,
        format_fps(video.frame_rate)
    );
    if video.rotation != 0 {
        video_text.push_str(&format!(", rotated {}°", video.rotation));
    }
    let audio_text = match &info.audio {
        None => "none".to_owned(),
        Some(audio) => {
            let mut text = audio.codec.clone();
            if let Some(rate) = audio.sample_rate {
                text.push_str(&format!(", {rate} Hz"));
            }
            if let Some(channels) = audio.channels {
                text.push_str(&format!(", {channels} ch"));
            }
            text
        }
    };
    let rows = [
        ("File", info.file_name()),
        (
            "Duration",
            format!(
                "{} ({} frames)",
                ClockStamp(info.duration),
                info.frame_count()
            ),
        ),
        ("Video", video_text),
        ("Audio", audio_text),
        ("Container", info.container.clone()),
    ];
    egui::Grid::new("media-info")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (key, value) in rows {
                ui.strong(key);
                ui.label(value);
                ui.end_row();
            }
        });
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
}
