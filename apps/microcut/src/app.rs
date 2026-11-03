//! Top-level UI state and frame loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use microcut_engine::Session;
use microcut_media::{FfmpegBackend, MediaBackend, MediaInfo};
use microcut_time::{ClockStamp, FrameRate};

use crate::loader::ProbeJob;
use crate::playback::{Playback, Step, stop_frame};
use crate::preview::Preview;
use crate::thumbs::Thumbnails;
use crate::timeline::Timeline;

/// File extensions offered in the open dialog. Anything ffmpeg reads can still be dropped.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mpg", "mpeg", "wmv", "flv",
    "3gp", "ogv", "mxf",
];

/// Picture size for playback before the preview has been laid out.
const FALLBACK_PLAY_SIZE: (u32, u32) = (1280, 720);

/// Frames-per-pixel factor of one + key press (− uses the inverse).
const KEY_ZOOM: f64 = 0.5;

const OPEN_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);

/// What is open right now.
enum Document {
    Empty,
    Probing(ProbeJob),
    Loaded(Session),
    Failed { path: PathBuf, message: String },
}

/// Root application state.
pub struct MicroCutApp {
    /// The media backend, or why it is unavailable (shown permanently).
    backend: Result<Arc<dyn MediaBackend>, String>,
    preview: Option<Preview>,
    thumbs: Option<Thumbnails>,
    timeline: Timeline,
    transport: Transport,
    document: Document,
    ctx: egui::Context,
}

/// Playback state.
#[derive(Default)]
struct Transport {
    playing: Option<Playback>,
    /// The cursor as playback last set it; any other value means the user moved it.
    cursor: i64,
    /// Continue playing from the cursor once the user lets go of the timeline.
    resume: bool,
    /// Why playback stopped early or plays without sound.
    message: Option<String>,
}

impl Transport {
    fn is_playing(&self) -> bool {
        self.playing.is_some() || self.resume
    }
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
        let thumbs = backend.as_ref().ok().map(|b| Thumbnails::new(b, ctx));
        let mut app = MicroCutApp {
            backend,
            preview,
            thumbs,
            timeline: Timeline::default(),
            transport: Transport::default(),
            document: Document::Empty,
            ctx: ctx.clone(),
        };
        if let Some(path) = initial {
            app.open(path);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        self.transport = Transport::default();
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
            Document::Loaded(session) => Some(&session.media().path),
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
                if let Some(thumbs) = &mut self.thumbs {
                    thumbs.reset();
                }
                self.timeline = Timeline::default();
                (
                    format!("MicroCut — {}", info.file_name()),
                    Document::Loaded(Session::new(Arc::new(info))),
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
        if let Document::Loaded(session) = &mut self.document {
            cursor_keys(ui, session);
        }
        let space = ui.memory(|m| m.focused().is_none())
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        if space {
            self.toggle_play();
        }
    }

    fn toggle_play(&mut self) {
        if self.transport.is_playing() {
            // The cursor stays on the frame shown last.
            self.transport.playing = None;
            self.transport.resume = false;
        } else {
            self.start_playback(true);
        }
    }

    /// Plays from the cursor. On the last frame: from the start of the range if `rewind`, else
    /// not at all (continuing after the user jumped to the end).
    fn start_playback(&mut self, rewind: bool) {
        let (Ok(backend), Document::Loaded(session)) = (&self.backend, &mut self.document) else {
            return;
        };
        let last = session.last_frame();
        let range = session.range();
        if session.cursor() >= last {
            if !rewind {
                return;
            }
            session.jump_cursor(range.start());
        }
        let from = session.cursor();
        let to = stop_frame(from, (range.start(), range.end()), last);
        let size = match self.preview.as_ref().map(Preview::size) {
            Some((w, h)) if w > 0 && h > 0 => (w, h),
            _ => FALLBACK_PLAY_SIZE,
        };
        self.transport.playing = Some(Playback::start(
            backend,
            session.media(),
            (from, to),
            size,
            &self.ctx,
        ));
        self.transport.cursor = from;
        self.transport.message = None;
    }

    /// Moves playback on to the frame its clock has reached (before the preview is drawn).
    fn step_playback(&mut self) {
        let Document::Loaded(session) = &mut self.document else {
            self.transport = Transport::default();
            return;
        };
        if let Some(playback) = &mut self.transport.playing {
            if session.cursor() != self.transport.cursor {
                // The user moved the cursor: continue from there (after a drag ends).
                self.transport.playing = None;
                self.transport.resume = true;
            } else {
                let step = playback.update();
                if let Some(note) = playback.audio_note() {
                    self.transport.message = Some(format!("no sound: {note}"));
                }
                match step {
                    Step::Show(frame, picture) => {
                        if let Some(preview) = &mut self.preview {
                            preview.present(&self.ctx, frame, &picture);
                        }
                        session.jump_cursor(frame);
                        self.transport.cursor = session.cursor();
                    }
                    Step::Wait => {}
                    Step::Finished => self.transport.playing = None,
                    Step::Failed(message) => {
                        self.transport.playing = None;
                        self.transport.message = Some(message);
                    }
                }
                self.ctx.request_repaint();
            }
        }
        if self.transport.resume && !self.timeline.is_dragging() {
            self.transport.resume = false;
            self.start_playback(false);
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
            let summary = match &self.document {
                Document::Loaded(session) => Some(media_summary(session.media())),
                _ => None,
            };
            let path = self.current_path().map(|p| p.display().to_string());
            // Media facts on the right; the path takes the rest and is shortened if needed.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(summary) = summary {
                    ui.label(summary);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if let Some(path) = path {
                        ui.add(egui::Label::new(path).truncate());
                    }
                });
            });
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
            Document::Loaded(session) => {
                if let Some(preview) = &mut self.preview {
                    preview.show(ui, session.media(), session.cursor());
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
        let mut play_clicked = false;
        if let Document::Loaded(session) = &mut self.document {
            let status = Status {
                preview: self.preview.as_ref(),
                playing: self.transport.is_playing(),
                message: self.transport.message.as_deref(),
            };
            let (timeline, thumbs) = (&mut self.timeline, self.thumbs.as_mut());
            egui::Panel::bottom("timeline")
                .resizable(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    play_clicked = status_line(ui, session, &status);
                    ui.add_space(4.0);
                    timeline.show(ui, session, thumbs);
                    ui.add_space(4.0);
                });
        }
        if play_clicked {
            self.toggle_play();
        }
        // After the timeline (which may move the cursor), before the preview is drawn.
        self.step_playback();
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0.0))
            .show(ui, |ui| {
                self.document_view(ui);
                drop_hint(ui);
            });
    }
}

/// What the status line shows besides the session.
struct Status<'a> {
    preview: Option<&'a Preview>,
    playing: bool,
    message: Option<&'a str>,
}

/// Play/pause button, cursor time and frame, the selected range, playback and decoder state.
/// Returns whether the play/pause button was clicked.
fn status_line(ui: &mut egui::Ui, session: &Session, status: &Status<'_>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        clicked = play_button(ui, status.playing).clicked();
        let rate = session.frame_rate();
        let cursor = session.cursor();
        let time = ClockStamp(rate.frame_to_tick(cursor));
        ui.monospace(format!(
            "{time}   frame {cursor} / {}",
            session.last_frame()
        ));
        ui.separator();
        let range = session.range();
        let (start, end) = (range.start_time(rate), range.end_time(rate));
        ui.monospace(format!(
            "range {} – {} ({})",
            ClockStamp(start),
            ClockStamp(end),
            ClockStamp(end.saturating_sub(start)),
        ))
        .on_hover_text(format!(
            "frames {} – {} ({} frames)",
            range.start(),
            range.end(),
            range.frame_count()
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(preview) = status.preview {
                if let Some(error) = preview.error() {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                } else if preview.is_busy() && !status.playing {
                    ui.spinner();
                }
            }
            if let Some(message) = status.message {
                ui.add(egui::Label::new(egui::RichText::new(message).weak()).truncate());
            }
        });
    });
    clicked
}

/// A button with a drawn play triangle or pause bars (the default fonts lack ⏸). It never takes
/// keyboard focus, so Space always means play/pause.
fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let size = egui::vec2(30.0, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_text(if playing {
        "Pause (Space)"
    } else {
        "Play from the cursor (Space)"
    });
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    painter.rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let color = visuals.fg_stroke.color;
    let c = rect.center();
    if playing {
        for dx in [-3.5, 3.5] {
            let bar = egui::Rect::from_center_size(c + egui::vec2(dx, 0.0), egui::vec2(3.5, 11.0));
            painter.rect_filled(bar, 0.0, color);
        }
    } else {
        let points = vec![
            c + egui::vec2(-4.0, -6.0),
            c + egui::vec2(-4.0, 6.0),
            c + egui::vec2(6.0, 0.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            points,
            color,
            egui::Stroke::NONE,
        ));
    }
    response
}

/// ←/→ one frame, Shift+←/→ one second, Home/End first/last frame; the timeline scrolls along.
/// Only when no text field or other widget has the keyboard.
fn cursor_keys(ui: &mut egui::Ui, session: &mut Session) {
    if ui.memory(|m| m.focused().is_some()) {
        return;
    }
    let second = frames_per_second(session.frame_rate());
    let (cursor, last) = (session.cursor(), session.last_frame());
    let target = ui.input_mut(|i| {
        let shift = egui::Modifiers::SHIFT;
        let none = egui::Modifiers::NONE;
        // Shift variants first: `consume_key(NONE, …)` would also match Shift+key.
        if i.consume_key(shift, egui::Key::ArrowLeft) {
            Some(cursor.saturating_sub(second))
        } else if i.consume_key(shift, egui::Key::ArrowRight) {
            Some(cursor.saturating_add(second))
        } else if i.consume_key(none, egui::Key::ArrowLeft) {
            Some(cursor.saturating_sub(1))
        } else if i.consume_key(none, egui::Key::ArrowRight) {
            Some(cursor.saturating_add(1))
        } else if i.consume_key(none, egui::Key::Home) {
            Some(0)
        } else if i.consume_key(none, egui::Key::End) {
            Some(last)
        } else {
            None
        }
    });
    if let Some(target) = target {
        session.jump_cursor(target);
    }

    // + / − zoom the timeline around the cursor.
    let zoom = ui.input_mut(|i| {
        let none = egui::Modifiers::NONE;
        let zoom_in =
            i.consume_key(none, egui::Key::Plus) || i.consume_key(none, egui::Key::Equals);
        let zoom_out = i.consume_key(none, egui::Key::Minus);
        match (zoom_in, zoom_out) {
            (true, false) => Some(KEY_ZOOM),
            (false, true) => Some(1.0 / KEY_ZOOM),
            _ => None,
        }
    });
    if let Some(factor) = zoom {
        let cursor = session.cursor() as f64 + 0.5;
        let view = session.view_mut();
        let anchor = view.x_of(cursor).clamp(0.0, view.width());
        view.zoom_at(anchor, factor);
        session.jump_cursor(session.cursor());
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
