//! Top-level UI state and frame loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Key, Modifiers};
use microcut_engine::{RangeEnd, Session};
use microcut_media::{Encoders, FfmpegBackend, MediaBackend, MediaInfo};
use microcut_time::{ClockStamp, FrameRate};

use crate::export::{self, Choices, ExportDialog, Outcome};
use crate::loader::ProbeJob;
use crate::playback::{Playback, Step, stop_frame};
use crate::preview::Preview;
use crate::sync::{self, SyncEdit};
use crate::thumbs::Thumbnails;
use crate::timeline::Timeline;
use crate::tracks;
use crate::volume::Volume;

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

const EXPORT_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::E);

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
    volume: Volume,
    document: Document,
    exports: Exports,
    ctx: egui::Context,
}

/// Export state.
#[derive(Default)]
struct Exports {
    /// The open export dialog.
    dialog: Option<ExportDialog>,
    /// Codec choices of the last dialog.
    choices: Choices,
    /// The encoders of the installed FFmpeg, asked once.
    encoders: Option<Encoders>,
    /// The file written last, shown in the status line.
    done: Option<PathBuf>,
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
            volume: Volume::default(),
            document: Document::Empty,
            exports: Exports::default(),
            ctx: ctx.clone(),
        };
        if let Some(path) = initial {
            app.open(path);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        self.transport = Transport::default();
        self.exports.done = None;
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

    /// Opens the export dialog for the selected range (playback pauses).
    fn open_export(&mut self) {
        let (Ok(backend), Document::Loaded(session)) = (&self.backend, &self.document) else {
            return;
        };
        self.transport.playing = None;
        self.transport.resume = false;
        let encoders = match &self.exports.encoders {
            Some(encoders) => encoders.clone(),
            None => match backend.encoders() {
                Ok(encoders) => self.exports.encoders.insert(encoders).clone(),
                Err(err) => {
                    self.transport.message = Some(format!("cannot export: {err}"));
                    return;
                }
            },
        };
        let media = Arc::clone(session.media());
        let cut = (session.range(), session.audio_delay_ms());
        match ExportDialog::new(media, cut, encoders, self.exports.choices) {
            Ok(dialog) => self.exports.dialog = Some(dialog),
            Err(message) => self.transport.message = Some(message),
        }
    }

    /// Shows the export dialog while it is open.
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Ok(backend)) = (&mut self.exports.dialog, &self.backend) else {
            return;
        };
        let outcome = dialog.show(ctx, backend);
        self.exports.choices = dialog.choices();
        if let Document::Loaded(session) = &mut self.document {
            session.set_audio_delay_ms(dialog.audio_delay_ms());
        }
        match outcome {
            Outcome::Open => {}
            Outcome::Closed => self.exports.dialog = None,
            Outcome::Exported(path) => {
                self.exports.dialog = None;
                self.exports.done = Some(path);
            }
        }
    }

    fn handle_input(&mut self, ui: &mut egui::Ui) {
        if self.exports.dialog.is_some() {
            // The dialog has the keyboard; files dropped meanwhile are ignored.
            return;
        }
        let free_keyboard = ui.memory(|m| m.focused().is_none());
        let export = ui.input_mut(|i| {
            i.consume_shortcut(&EXPORT_SHORTCUT) || (free_keyboard && i.consume_key(NONE, Key::E))
        });
        if export {
            self.open_export();
            return;
        }
        let dropped = ui.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(path);
        }
        if ui.input_mut(|i| i.consume_shortcut(&OPEN_SHORTCUT)) {
            self.open_dialog();
        }
        let moved = match &mut self.document {
            Document::Loaded(session) => {
                mark_keys(ui, session);
                cursor_keys(ui, session)
            }
            _ => false,
        };
        if moved {
            // Navigating with the keyboard pauses playback (on the frame the key chose).
            self.transport.playing = None;
            self.transport.resume = false;
        }
        let space = ui.memory(|m| m.focused().is_none())
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
        if space {
            self.toggle_play();
        }
    }

    /// Switches to audio track `track`; a running playback continues from the cursor with it.
    fn select_audio_track(&mut self, track: usize) {
        let Document::Loaded(session) = &mut self.document else {
            return;
        };
        if session.set_audio_track(track) && self.transport.playing.is_some() {
            self.transport.playing = None;
            self.start_playback(false);
        }
    }

    /// Changes the audio sync; once the user is done with it, a running playback continues from
    /// the cursor with the new value.
    fn set_audio_delay(&mut self, edit: SyncEdit) {
        let Document::Loaded(session) = &mut self.document else {
            return;
        };
        session.set_audio_delay_ms(edit.ms);
        if edit.commit && self.transport.playing.is_some() {
            self.transport.playing = None;
            self.start_playback(false);
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
            (
                session.audio_track(),
                session.audio_delay(),
                &self.volume.gain(),
            ),
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
                        // The timeline follows the cursor, unless the user is moving it.
                        if self.timeline.is_dragging() {
                            session.set_cursor(frame);
                        } else {
                            session.jump_cursor(frame);
                        }
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
            let loaded = matches!(self.document, Document::Loaded(_)) && self.backend.is_ok();
            let export = ui
                .add_enabled(loaded, egui::Button::new("Export…"))
                .on_hover_text(format!(
                    "Export the selected range (E or {})",
                    ui.ctx().format_shortcut(&EXPORT_SHORTCUT)
                ));
            if export.clicked() {
                export.surrender_focus();
                self.open_export();
            }
            if let Some(path) = &self.exports.done {
                ui.add_space(8.0);
                ui.label(format!("Exported {}", display_name(path)));
                let hint = path.display().to_string();
                if quiet_button(ui, "Show in folder", &hint).clicked()
                    && let Err(message) = export::show_in_folder(path)
                {
                    self.transport.message = Some(message);
                }
                ui.add_space(8.0);
            }
            let summary = match &self.document {
                Document::Loaded(session) => {
                    Some(media_summary(session.media(), session.audio_track()))
                }
                _ => None,
            };
            let path = self.current_path().map(|p| p.display().to_string());
            // Media facts on the right; the path takes the rest and is shortened if needed.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(summary) = summary {
                    ui.add(egui::Label::new(summary).truncate());
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // A sliver of path would only show "…".
                    if let Some(path) = path.filter(|_| ui.available_width() > 60.0) {
                        ui.add(egui::Label::new(path).truncate());
                    }
                });
            });
        });
        if let Err(message) = &self.backend {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
    }

    /// The central area. Returns whether the preview picture was clicked.
    fn document_view(&mut self, ui: &mut egui::Ui) -> bool {
        let mut clicked = false;
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
                    clicked = preview.show(ui, session.media(), session.cursor());
                }
            }
            Document::Failed { path, message } => {
                ui.heading(format!("Could not open {}", display_name(path)));
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
        }
        clicked
    }
}

impl eframe::App for MicroCutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_input(ui);
        self.poll_probe();

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        let mut actions = StatusActions::default();
        if let Document::Loaded(session) = &mut self.document {
            let status = Status {
                preview: self.preview.as_ref(),
                playing: self.transport.is_playing(),
                message: self.transport.message.as_deref(),
            };
            let (timeline, thumbs) = (&mut self.timeline, self.thumbs.as_mut());
            let volume = &mut self.volume;
            egui::Panel::bottom("timeline")
                .resizable(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    actions = status_line(ui, session, &status, volume);
                    ui.add_space(4.0);
                    timeline.show(ui, session, thumbs);
                    ui.add_space(4.0);
                });
        }
        if actions.play {
            self.toggle_play();
        }
        if let Some(track) = actions.track {
            self.select_audio_track(track);
        }
        if let Some(edit) = actions.sync {
            self.set_audio_delay(edit);
        }
        if let (Some(which), Document::Loaded(session)) = (actions.mark, &mut self.document) {
            match which {
                RangeEnd::Start => session.mark_start(),
                RangeEnd::End => session.mark_end(),
            }
        }
        // After the timeline (which may move the cursor), before the preview is drawn.
        self.step_playback();
        let preview_clicked = egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(0.0))
            .show(ui, |ui| {
                let clicked = self.document_view(ui);
                drop_hint(ui);
                clicked
            })
            .inner;
        // A click on the picture plays or pauses (takes effect on the next repaint).
        if preview_clicked {
            self.toggle_play();
            self.ctx.request_repaint();
        }
        let ctx = self.ctx.clone();
        self.export_dialog(&ctx);
    }
}

/// What the status line shows besides the session.
struct Status<'a> {
    preview: Option<&'a Preview>,
    playing: bool,
    message: Option<&'a str>,
}

/// What the user asked for in the status line.
#[derive(Default)]
struct StatusActions {
    /// The play/pause button was clicked.
    play: bool,
    /// This audio track was chosen.
    track: Option<usize>,
    /// The audio sync was changed.
    sync: Option<SyncEdit>,
    /// Set this end of the range to the cursor.
    mark: Option<RangeEnd>,
}

/// Play/pause button, cursor time and frame, the selected range, playback and decoder state,
/// audio sync, audio tracks, volume.
fn status_line(
    ui: &mut egui::Ui,
    session: &Session,
    status: &Status<'_>,
    volume: &mut Volume,
) -> StatusActions {
    let mut actions = StatusActions::default();
    // The right part first, so the left part (range, messages) is shortened when space is tight.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            volume.ui(ui);
            ui.add_space(6.0);
            actions.track = tracks::buttons(ui, &session.media().audio, session.audio_track());
            if session.audio_track().is_some() {
                ui.add_space(6.0);
                actions.sync = sync::control(ui, session.audio_delay_ms());
                ui.weak("Sync");
            }
            ui.add_space(8.0);
            let preview_error = status.preview.and_then(Preview::error);
            if status.preview.is_some_and(Preview::is_busy)
                && !status.playing
                && preview_error.is_none()
            {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                actions.play = play_button(ui, status.playing).clicked();
                let rate = session.frame_rate();
                let cursor = session.cursor();
                let time = ClockStamp(rate.frame_to_tick(cursor));
                ui.monospace(format!(
                    "{time}   frame {cursor} / {}",
                    session.last_frame()
                ));
                ui.separator();
                if quiet_button(ui, "Set start", "Start the range at the cursor (I)").clicked() {
                    actions.mark = Some(RangeEnd::Start);
                }
                if quiet_button(ui, "Set end", "End the range at the cursor, included (O)")
                    .clicked()
                {
                    actions.mark = Some(RangeEnd::End);
                }
                let range = session.range();
                let (start, end) = (range.start_time(rate), range.end_time(rate));
                let text = format!(
                    "range {} – {} ({})",
                    ClockStamp(start),
                    ClockStamp(end),
                    ClockStamp(end.saturating_sub(start)),
                );
                ui.add(egui::Label::new(egui::RichText::new(text).monospace()).truncate())
                    .on_hover_text(format!(
                        "frames {} – {} ({} frames)",
                        range.start(),
                        range.end(),
                        range.frame_count()
                    ));
                if let Some(error) = preview_error {
                    let text = egui::RichText::new(error).color(ui.visuals().error_fg_color);
                    ui.add(egui::Label::new(text).truncate());
                }
                if let Some(message) = status.message {
                    ui.add(egui::Label::new(egui::RichText::new(message).weak()).truncate());
                }
            });
        })
    });
    actions
}

/// A small button that gives the keyboard back right away, so Space and the navigation keys
/// keep working after a click.
fn quiet_button(ui: &mut egui::Ui, text: &str, hint: &str) -> egui::Response {
    let response = ui.small_button(text).on_hover_text(hint);
    if response.has_focus() || response.gained_focus() {
        response.surrender_focus();
    }
    response
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

/// Where a navigation key moves the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jump {
    /// By this many frames (negative: back).
    Frames(i64),
    /// By this many seconds (rounded to whole frames).
    Seconds(i64),
    /// First frame of the range.
    RangeStart,
    /// Last frame of the range.
    RangeEnd,
    /// First frame of the clip.
    First,
    /// Last frame of the clip.
    Last,
}

/// Navigation keys. Shift variants come first: `consume_key(NONE, …)` also matches Shift+key.
/// WASD work like the arrow keys.
const NAVIGATION: &[(Modifiers, Key, Jump)] = &[
    (SHIFT, Key::ArrowLeft, Jump::Seconds(-1)),
    (SHIFT, Key::ArrowRight, Jump::Seconds(1)),
    (SHIFT, Key::A, Jump::Seconds(-1)),
    (SHIFT, Key::D, Jump::Seconds(1)),
    (NONE, Key::ArrowLeft, Jump::Frames(-1)),
    (NONE, Key::ArrowRight, Jump::Frames(1)),
    (NONE, Key::ArrowUp, Jump::Seconds(1)),
    (NONE, Key::ArrowDown, Jump::Seconds(-1)),
    (NONE, Key::A, Jump::Frames(-1)),
    (NONE, Key::D, Jump::Frames(1)),
    (NONE, Key::W, Jump::Seconds(1)),
    (NONE, Key::S, Jump::Seconds(-1)),
    (NONE, Key::PageUp, Jump::Seconds(10)),
    (NONE, Key::PageDown, Jump::Seconds(-10)),
    (COMMAND, Key::Home, Jump::First),
    (COMMAND, Key::End, Jump::Last),
    (NONE, Key::Home, Jump::RangeStart),
    (NONE, Key::End, Jump::RangeEnd),
];
const NONE: Modifiers = Modifiers::NONE;
const SHIFT: Modifiers = Modifiers::SHIFT;
const COMMAND: Modifiers = Modifiers::COMMAND;

/// Where the cursor can jump to besides relative moves.
struct Landmarks {
    /// First and last frame of the range.
    range: (i64, i64),
    last: i64,
    frames_per_second: i64,
}

/// The frame `jump` leads to from `cursor` (clamping to the clip is the session's job).
fn jump_target(jump: Jump, cursor: i64, marks: &Landmarks) -> i64 {
    match jump {
        Jump::Frames(n) => cursor.saturating_add(n),
        Jump::Seconds(n) => cursor.saturating_add(n.saturating_mul(marks.frames_per_second)),
        Jump::RangeStart => marks.range.0,
        Jump::RangeEnd => marks.range.1,
        Jump::First => 0,
        Jump::Last => marks.last,
    }
}

/// ←/→ (A/D) one frame, ↑/↓ (W/S, Shift+←/→) one second, Page Up/Down ten seconds, Home/End
/// start/end of the range, Ctrl+Home/End first/last frame; the timeline scrolls along. +/− zoom.
/// Only when no text field or other widget has the keyboard. Returns whether the cursor was moved.
fn cursor_keys(ui: &mut egui::Ui, session: &mut Session) -> bool {
    if ui.memory(|m| m.focused().is_some()) {
        return false;
    }
    let marks = Landmarks {
        range: (session.range().start(), session.range().end()),
        last: session.last_frame(),
        frames_per_second: frames_per_second(session.frame_rate()),
    };
    let cursor = session.cursor();
    let jump = ui.input_mut(|i| {
        NAVIGATION
            .iter()
            .find(|(modifiers, key, _)| i.consume_key(*modifiers, *key))
            .map(|(_, _, jump)| *jump)
    });
    if let Some(jump) = jump {
        session.jump_cursor(jump_target(jump, cursor, &marks));
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
    jump.is_some()
}

/// `I` starts and `O` ends the range at the cursor (only when no widget has the keyboard).
fn mark_keys(ui: &mut egui::Ui, session: &mut Session) {
    if ui.memory(|m| m.focused().is_some()) {
        return;
    }
    let (start, end) = ui.input_mut(|i| (i.consume_key(NONE, Key::I), i.consume_key(NONE, Key::O)));
    if start {
        session.mark_start();
    }
    if end {
        session.mark_end();
    }
}

/// Frames in one second, rounded (at least 1).
fn frames_per_second(rate: FrameRate) -> i64 {
    let (num, den) = (i64::from(rate.num()), i64::from(rate.den()));
    ((num + den / 2) / den).max(1)
}

/// `h264 1280×720 · 60 fps · aac 48 kHz stereo · 00:08:55.300`, describing the selected audio
/// track (`· 2 audio tracks` when there are several).
fn media_summary(info: &MediaInfo, track: Option<usize>) -> String {
    let video = &info.video;
    let (w, h) = video.display_size();
    let mut parts = vec![
        format!("{} {w}×{h}", video.codec),
        format_fps(video.frame_rate),
    ];
    if let Some(audio) = track.and_then(|t| info.audio.get(t)) {
        parts.push(tracks::describe(audio));
    }
    if info.audio.len() > 1 {
        parts.push(format!("{} audio tracks", info.audio.len()));
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
    fn navigation_jumps() {
        let marks = Landmarks {
            range: (300, 2000),
            last: 5000,
            frames_per_second: 60,
        };
        let target = |jump| jump_target(jump, 1000, &marks);
        assert_eq!(target(Jump::Frames(-1)), 999);
        assert_eq!(target(Jump::Seconds(1)), 1060);
        assert_eq!(target(Jump::Seconds(-10)), 400);
        assert_eq!(target(Jump::RangeStart), 300);
        assert_eq!(target(Jump::RangeEnd), 2000);
        assert_eq!(target(Jump::First), 0);
        assert_eq!(target(Jump::Last), 5000);
        assert_eq!(
            jump_target(Jump::Seconds(10), i64::MAX - 5, &marks),
            i64::MAX
        );
    }

    #[test]
    fn keys_are_bound_once() {
        for (n, (modifiers, key, _)) in NAVIGATION.iter().enumerate() {
            let twice = NAVIGATION
                .iter()
                .skip(n + 1)
                .any(|(m, k, _)| m == modifiers && k == key);
            assert!(!twice, "{modifiers:?} {key:?} bound twice");
        }
    }

    #[test]
    fn one_second_in_frames() {
        let fps = |n, d| frames_per_second(FrameRate::new(n, d).unwrap());
        assert_eq!(fps(60, 1), 60);
        assert_eq!(fps(30_000, 1_001), 30);
        assert_eq!(fps(1, 10), 1);
    }
}
