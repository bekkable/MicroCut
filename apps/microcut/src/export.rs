//! The export dialog: codecs, quality, audio track, file name; then the running export with
//! progress and cancel.

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui;
use microcut_engine::{
    FrameRange, clean_file_name, default_audio_track, default_file_name, estimate_bytes,
    free_file_name,
};
use microcut_media::{
    AudioCodec, AudioExport, Encoders, ExportEnd, ExportJob, MediaBackend, MediaError, MediaInfo,
    VideoCodec, VideoQuality,
};
use microcut_time::{ClockStamp, Tick};

use crate::report::{self, Report};
use crate::{sync, tracks};

/// Codec choices, remembered from one export to the next and between runs (the app saves them
/// with eframe's storage). File name, audio track and sync are not: they belong to one video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choices {
    pub video: VideoCodec,
    pub quality: VideoQuality,
    pub audio: AudioCodec,
    pub kbps: u32,
    /// Hardware acceleration wanted (used only where possible, see `hardware_in_use`).
    pub hardware: bool,
}

impl Default for Choices {
    fn default() -> Choices {
        Choices {
            video: VideoCodec::H264,
            quality: VideoQuality::High,
            audio: AudioCodec::Aac,
            kbps: AudioCodec::Aac.default_bitrate(),
            hardware: false,
        }
    }
}

impl Choices {
    /// Whether the export uses hardware acceleration: wanted, an NVIDIA encoder for the chosen
    /// codec exists (`hardware_codecs`), and the video needs no rotation.
    fn hardware_in_use(&self, hardware_codecs: &[VideoCodec], rotated: bool) -> bool {
        self.hardware && hardware_codecs.contains(&self.video) && !rotated
    }

    /// The choices as saved between runs: `video=h264;quality=high;audio=aac;kbps=192;hardware=0`.
    /// The names stay the same when labels change; [`Choices::from_setting`] reads them back.
    pub fn to_setting(self) -> String {
        let video = match self.video {
            VideoCodec::H264 => "h264",
            VideoCodec::H265 => "h265",
            VideoCodec::Av1 => "av1",
        };
        let quality = match self.quality {
            VideoQuality::High => "high",
            VideoQuality::Medium => "medium",
            VideoQuality::Small => "small",
        };
        let audio = match self.audio {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
        };
        format!(
            "video={video};quality={quality};audio={audio};kbps={};hardware={}",
            self.kbps,
            u8::from(self.hardware)
        )
    }

    /// Reads [`Choices::to_setting`]. The file may be old, edited or broken: anything unknown or
    /// missing keeps its default, and a bit rate the audio codec does not offer becomes its
    /// default bit rate.
    pub fn from_setting(text: &str) -> Choices {
        let mut choices = Choices::default();
        let mut kbps = None;
        for (key, value) in text.split(';').filter_map(|pair| pair.split_once('=')) {
            match (key.trim(), value.trim()) {
                ("video", "h264") => choices.video = VideoCodec::H264,
                ("video", "h265") => choices.video = VideoCodec::H265,
                ("video", "av1") => choices.video = VideoCodec::Av1,
                ("quality", "high") => choices.quality = VideoQuality::High,
                ("quality", "medium") => choices.quality = VideoQuality::Medium,
                ("quality", "small") => choices.quality = VideoQuality::Small,
                ("audio", "aac") => choices.audio = AudioCodec::Aac,
                ("audio", "opus") => choices.audio = AudioCodec::Opus,
                ("kbps", value) => kbps = value.parse::<u32>().ok(),
                ("hardware", "1") => choices.hardware = true,
                ("hardware", "0") => choices.hardware = false,
                _ => {}
            }
        }
        choices.kbps = kbps
            .filter(|k| choices.audio.bitrates().contains(k))
            .unwrap_or(choices.audio.default_bitrate());
        choices
    }

    /// The same choices, with codecs the installed FFmpeg lacks replaced by available ones.
    fn available_in(mut self, encoders: &Encoders) -> Choices {
        if !encoders.video.contains(&self.video)
            && let Some(first) = encoders.video.first()
        {
            self.video = *first;
        }
        if !encoders.audio.contains(&self.audio)
            && let Some(first) = encoders.audio.first()
        {
            self.audio = *first;
            self.kbps = first.default_bitrate();
        }
        if !self.audio.bitrates().contains(&self.kbps) {
            self.kbps = self.audio.default_bitrate();
        }
        self
    }
}

/// What the dialog wants after a repaint.
pub enum Outcome {
    /// Stay open.
    Open,
    /// Closed (after an export or without one).
    Closed,
}

/// A message under the settings.
enum Note {
    Info(String),
    Error(String),
}

/// The export dialog of one video and range.
pub struct ExportDialog {
    media: Arc<MediaInfo>,
    range: FrameRange,
    encoders: Encoders,
    choices: Choices,
    /// `None`: export without sound.
    track: Option<usize>,
    /// Audio sync in milliseconds (shared with the status line, see [`ExportDialog::audio_delay_ms`]).
    delay_ms: i32,
    /// The file name as typed.
    name: String,
    note: Option<Note>,
    running: Option<ExportTask>,
    /// The export is done: the dialog shows what was written.
    finished: Option<Box<Report>>,
    /// Set after the first frame: the Enter that opened the dialog must not also encode.
    armed: bool,
}

impl ExportDialog {
    /// A dialog for `range` of `media` with audio sync `delay_ms`, starting from the `choices`
    /// of the last export.
    pub fn new(
        media: Arc<MediaInfo>,
        (range, delay_ms): (FrameRange, i32),
        encoders: Encoders,
        choices: Choices,
    ) -> Result<ExportDialog, String> {
        if encoders.video.is_empty() {
            return Err(
                "Your FFmpeg has none of the video encoders MicroCut uses (libx264, libx265, \
                 libsvtav1). Install a full FFmpeg build to export."
                    .to_owned(),
            );
        }
        let choices = choices.available_in(&encoders);
        let track = if encoders.audio.is_empty() {
            None
        } else {
            default_audio_track(&media.audio)
        };
        let name = default_file_name(&media, range);
        Ok(ExportDialog {
            media,
            range,
            encoders,
            choices,
            track,
            delay_ms,
            name,
            note: None,
            running: None,
            finished: None,
            armed: false,
        })
    }

    /// The codec choices, to start the next dialog with.
    pub fn choices(&self) -> Choices {
        self.choices
    }

    /// The audio sync as set in the dialog (it changes the one of the session too).
    pub fn audio_delay_ms(&self) -> i32 {
        self.delay_ms
    }

    /// The folder the export goes to: that of the video.
    fn dir(&self) -> &Path {
        self.media.path.parent().unwrap_or(Path::new("."))
    }

    /// Shows the dialog; starts, follows and ends the export.
    pub fn show(&mut self, ctx: &egui::Context, backend: &Arc<dyn MediaBackend>) -> Outcome {
        self.poll_export();
        if let Some(report) = self.finished.take() {
            let outcome = self.show_report(ctx, &report);
            self.finished = Some(report);
            return outcome;
        }
        let modal = egui::Modal::new(egui::Id::unique("export")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Export");
            ui.add_space(8.0);
            let running = self.running.is_some();
            ui.add_enabled_ui(!running, |ui| self.settings(ui));
            ui.add_space(8.0);
            ui.separator();
            let target = self.target_name(ui);
            ui.add_space(8.0);
            if let Some(task) = &self.running {
                progress(ui, task);
            } else if let Some(note) = &self.note {
                match note {
                    Note::Info(text) => ui.weak(text),
                    Note::Error(text) => ui.colored_label(ui.visuals().error_fg_color, text),
                };
            }
            ui.add_space(8.0);
            self.buttons(ui, target.as_deref(), backend)
        });
        let close = modal.should_close();
        let outcome = modal.inner;
        self.armed = true;
        if self.running.is_none() && close {
            return Outcome::Closed;
        }
        outcome
    }

    /// Codec, quality and audio rows, and what the export will contain.
    fn settings(&mut self, ui: &mut egui::Ui) {
        let hardware_codecs = &self.encoders.hardware;
        let rotated = self.media.video.rotation != 0;
        let choices = &mut self.choices;
        egui::Grid::new("export settings")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Video");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("video codec")
                        .selected_text(choices.video.label())
                        .show_ui(ui, |ui| {
                            for codec in &self.encoders.video {
                                ui.selectable_value(&mut choices.video, *codec, codec.label());
                            }
                        })
                        .response
                        .on_hover_text(video_hint(choices.video));
                    let hardware = choices.hardware_in_use(hardware_codecs, rotated);
                    let label = |q: VideoQuality| {
                        if hardware {
                            format!("{} (CQ {})", q.label(), choices.video.hardware_cq(q))
                        } else {
                            format!("{} (CRF {})", q.label(), choices.video.crf(q))
                        }
                    };
                    egui::ComboBox::from_id_salt("video quality")
                        .selected_text(label(choices.quality))
                        .show_ui(ui, |ui| {
                            for quality in VideoQuality::ALL {
                                let text = label(quality);
                                ui.selectable_value(&mut choices.quality, quality, text);
                            }
                        });
                });
                ui.end_row();

                ui.label("");
                let possible = hardware_codecs.contains(&choices.video) && !rotated;
                let hint = if rotated {
                    "Not for rotated videos."
                } else if !possible {
                    "Your FFmpeg has no NVIDIA encoder for this codec."
                } else {
                    "Decodes on the graphics card (CUDA) and encodes with NVENC: much faster, \
                     same quality at about the same file size. Needs an NVIDIA graphics card."
                };
                let mut shown = choices.hardware && possible;
                let checkbox = ui
                    .add_enabled(
                        possible,
                        egui::Checkbox::new(&mut shown, "Hardware acceleration (NVIDIA)"),
                    )
                    .on_hover_text(hint)
                    .on_disabled_hover_text(hint);
                if checkbox.changed() {
                    choices.hardware = shown;
                }
                ui.end_row();

                ui.label("Audio");
                ui.vertical(|ui| {
                    let audio = &self.media.audio;
                    let track_label = |track: Option<usize>| match track {
                        Some(t) => audio
                            .get(t)
                            .map(|a| tracks::tooltip(t, a))
                            .unwrap_or_default(),
                        None => "No audio".to_owned(),
                    };
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(self.track.is_some(), |ui| {
                            ui.weak("Sync");
                            if let Some(edit) = sync::control(ui, self.delay_ms) {
                                self.delay_ms = edit.ms;
                            }
                        });
                        ui.add_space(4.0);
                        ui.add_enabled_ui(!self.encoders.audio.is_empty(), |ui| {
                            egui::ComboBox::from_id_salt("audio track")
                                .width(ui.available_width().max(120.0))
                                .selected_text(track_label(self.track))
                                .show_ui(ui, |ui| {
                                    for track in (0..audio.len()).map(Some).chain([None]) {
                                        let text = track_label(track);
                                        ui.selectable_value(&mut self.track, track, text);
                                    }
                                });
                        });
                    });
                    if self.track.is_some() {
                        ui.horizontal(|ui| {
                            let before = choices.audio;
                            egui::ComboBox::from_id_salt("audio codec")
                                .selected_text(choices.audio.label())
                                .show_ui(ui, |ui| {
                                    for codec in &self.encoders.audio {
                                        ui.selectable_value(&mut choices.audio, *codec, codec.label());
                                    }
                                });
                            if choices.audio != before {
                                choices.kbps = choices.audio.default_bitrate();
                            }
                            egui::ComboBox::from_id_salt("audio bitrate")
                                .selected_text(format!("{} kbit/s", choices.kbps))
                                .show_ui(ui, |ui| {
                                    for kbps in choices.audio.bitrates() {
                                        let text = format!("{kbps} kbit/s");
                                        ui.selectable_value(&mut choices.kbps, *kbps, text);
                                    }
                                });
                        });
                    } else if self.encoders.audio.is_empty() && !audio.is_empty() {
                        ui.weak("Your FFmpeg has no AAC or Opus encoder.");
                    }
                });
                ui.end_row();

                ui.label("Result");
                let frames = self.range.frame_count();
                let rate = self.media.video.frame_rate;
                let duration = self
                    .range
                    .end_time(rate)
                    .saturating_sub(self.range.start_time(rate));
                let kbps = self.track.map(|_| choices.kbps);
                let size = estimate_bytes(
                    &self.media,
                    frames,
                    (
                        choices.video,
                        choices.quality,
                        choices.hardware_in_use(hardware_codecs, rotated),
                    ),
                    kbps,
                );
                ui.label(format!(
                    "{frames} frames · {} · ≈ {}",
                    ClockStamp(duration),
                    format_size(size)
                ))
                .on_hover_text(
                    "The size is an estimate: how well a video compresses depends on what it shows.",
                );
                ui.end_row();
            });
    }

    /// The file name field with its checks. Returns the name to write, if it is valid.
    fn target_name(&mut self, ui: &mut egui::Ui) -> Option<String> {
        let running = self.running.is_some();
        ui.horizontal(|ui| {
            ui.label("File");
            ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY),
            );
        });
        let dir = self.dir().to_path_buf();
        ui.add(
            egui::Label::new(egui::RichText::new(format!("in {}", dir.display())).weak())
                .truncate(),
        );
        let checked = clean_file_name(&self.name).and_then(|name| {
            free_file_name(&dir, &name, |p| p.exists())
                .map(|free| (name, free))
                .ok_or("Too many files with this name. Choose another one.")
        });
        match checked {
            Ok((name, free)) => {
                if free != name && !running {
                    ui.weak(format!(
                        "{name} exists already; the export is saved as {free}."
                    ));
                }
                Some(free)
            }
            Err(problem) => {
                ui.colored_label(ui.visuals().error_fg_color, problem);
                None
            }
        }
    }

    /// Cancel and Encode (or Cancel of the running export). Enter encodes.
    fn buttons(
        &mut self,
        ui: &mut egui::Ui,
        target: Option<&str>,
        backend: &Arc<dyn MediaBackend>,
    ) -> Outcome {
        let mut outcome = Outcome::Open;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(task) = &self.running {
                if ui.button("Cancel").clicked() {
                    task.cancel();
                }
                return;
            }
            let encode = ui
                .add_enabled(target.is_some(), egui::Button::new("Encode"))
                .on_hover_text("Start the export (Enter)");
            let enter = self.armed && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(target) = target
                && (encode.clicked() || enter)
            {
                self.start(backend, target, ui.ctx());
            }
            if ui.button("Cancel").clicked() {
                outcome = Outcome::Closed;
            }
        });
        outcome
    }

    fn start(&mut self, backend: &Arc<dyn MediaBackend>, target: &str, ctx: &egui::Context) {
        let job = ExportJob {
            output: self.dir().join(target),
            from: self.range.start(),
            count: self.range.frame_count(),
            video: self.choices.video,
            quality: self.choices.quality,
            hardware: self
                .choices
                .hardware_in_use(&self.encoders.hardware, self.media.video.rotation != 0),
            audio: self.track.map(|track| AudioExport {
                track,
                codec: self.choices.audio,
                kbps: self.choices.kbps,
                delay: Tick::from_millis(i64::from(self.delay_ms)),
            }),
        };
        self.note = None;
        self.running = Some(ExportTask::start(
            Arc::clone(backend),
            Arc::clone(&self.media),
            job,
            ctx.clone(),
        ));
    }

    /// Ends the running export once its result is in.
    fn poll_export(&mut self) {
        let Some(result) = self.running.as_ref().and_then(ExportTask::poll) else {
            return;
        };
        self.running = None;
        match result {
            Ok(Ended::Finished(report)) => {
                self.note = None;
                self.finished = Some(report);
            }
            Ok(Ended::Cancelled) => {
                self.note = Some(Note::Info("Export cancelled.".to_owned()));
            }
            Err(message) => self.note = Some(Note::Error(message)),
        }
    }

    /// The finished export: file name, facts about the file and the encoding, Show in folder and
    /// Close (Enter, Escape).
    fn show_report(&mut self, ctx: &egui::Context, report: &Report) -> Outcome {
        let mut outcome = Outcome::Open;
        let modal = egui::Modal::new(egui::Id::unique("export finished")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading("Export finished");
            ui.add_space(8.0);
            report::show(ui, report, &self.media);
            ui.add_space(8.0);
            if let Some(Note::Error(text)) = &self.note {
                ui.colored_label(ui.visuals().error_fg_color, text);
                ui.add_space(8.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.button("Close");
                if close.clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome = Outcome::Closed;
                }
                let folder = ui
                    .button("Show in folder")
                    .on_hover_text(report.job.output.display().to_string());
                if folder.clicked()
                    && let Err(message) = show_in_folder(&report.job.output)
                {
                    self.note = Some(Note::Error(capitalize(&message)));
                }
            });
        });
        if modal.should_close() {
            outcome = Outcome::Closed;
        }
        outcome
    }
}

fn video_hint(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "Plays everywhere.",
        VideoCodec::H265 => {
            "About a third smaller than H.264; slower to encode, not every player has it."
        }
        VideoCodec::Av1 => "Smallest files; newer players and browsers only.",
    }
}

/// Progress bar, frames and time left of the running export.
fn progress(ui: &mut egui::Ui, task: &ExportTask) {
    let done = task.frames_done();
    let fraction = if task.total > 0 {
        (done as f64 / task.total as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    ui.add(egui::ProgressBar::new(fraction).show_percentage());
    let mut text = format!("{done} / {} frames", task.total);
    if let Some(fps) = task.frames_per_second() {
        text.push_str(&format!(" · {fps:.0} fps"));
    }
    if let Some(left) = task.time_left() {
        text.push_str(&format!(" · about {} left", format_duration(left)));
    }
    ui.weak(text);
}

/// `850 KB`, `4.4 MB`, `1.2 GB` (decimal units, as file managers show them).
pub(crate) fn format_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1e6 {
        format!("{:.0} KB", (b / 1e3).max(1.0))
    } else if b < 1e9 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.2} GB", b / 1e9)
    }
}

/// `45 s`, `3 min 05 s`, `1 h 02 min`.
pub(crate) fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min {:02} s", secs / 60, secs % 60)
    } else {
        format!("{} h {:02} min", secs / 3600, (secs / 60) % 60)
    }
}

/// How an export ended without an error.
enum Ended {
    Finished(Box<Report>),
    Cancelled,
}

/// An export running on a worker thread. Dropping it cancels the export and waits for ffmpeg to
/// stop (the partial file is removed).
struct ExportTask {
    total: i64,
    started: Instant,
    done: Arc<AtomicI64>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Ended, String>>,
    worker: Option<JoinHandle<()>>,
}

impl ExportTask {
    fn start(
        backend: Arc<dyn MediaBackend>,
        media: Arc<MediaInfo>,
        job: ExportJob,
        ctx: egui::Context,
    ) -> ExportTask {
        let (tx, rx) = mpsc::channel();
        let done = Arc::new(AtomicI64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let total = job.count;
        let (worker_done, worker_cancel, worker_tx) =
            (Arc::clone(&done), Arc::clone(&cancel), tx.clone());
        let spawned = thread::Builder::new()
            .name("microcut-export".to_owned())
            .spawn(move || {
                let mut progress = |frames: i64| {
                    worker_done.store(frames, Ordering::Relaxed);
                    ctx.request_repaint();
                    !worker_cancel.load(Ordering::Relaxed)
                };
                let run = || {
                    let started = Instant::now();
                    match backend.export(&media, &job, &mut progress)? {
                        ExportEnd::Cancelled => Ok(Ended::Cancelled),
                        ExportEnd::Finished => {
                            let elapsed = started.elapsed();
                            // What was really written; the report falls back to the job.
                            let bytes = std::fs::metadata(&job.output).ok().map(|m| m.len());
                            let written = backend.probe(&job.output).ok();
                            Ok(Ended::Finished(Box::new(Report {
                                job,
                                elapsed,
                                bytes,
                                written,
                            })))
                        }
                    }
                };
                let result = match panic::catch_unwind(AssertUnwindSafe(run)) {
                    Ok(result) => result.map_err(|err: MediaError| capitalize(&err.to_string())),
                    Err(_) => Err("Internal error during the export.".to_owned()),
                };
                let _ = worker_tx.send(result);
                ctx.request_repaint();
            });
        let worker = match spawned {
            Ok(handle) => Some(handle),
            Err(err) => {
                let _ = tx.send(Err(format!("Could not start the export: {err}")));
                None
            }
        };
        ExportTask {
            total,
            started: Instant::now(),
            done,
            cancel,
            rx,
            worker,
        }
    }

    fn poll(&self) -> Option<Result<Ended, String>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("The export stopped unexpectedly.".to_owned()))
            }
        }
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn frames_done(&self) -> i64 {
        self.done
            .load(Ordering::Relaxed)
            .clamp(0, self.total.max(0))
    }

    /// Frames encoded per second so far, once there is enough to go by.
    fn frames_per_second(&self) -> Option<f64> {
        let done = self.frames_done();
        let secs = self.started.elapsed().as_secs_f64();
        (done > 0 && secs >= 0.5).then(|| done as f64 / secs)
    }

    /// Estimated from the speed so far, once there is enough to go by.
    fn time_left(&self) -> Option<Duration> {
        let done = self.frames_done();
        let elapsed = self.started.elapsed();
        if done < 1 || elapsed < Duration::from_secs(1) {
            return None;
        }
        let left = self.total.saturating_sub(done) as f64;
        let secs = elapsed.as_secs_f64() * left / done as f64;
        Duration::try_from_secs_f64(secs).ok()
    }
}

impl Drop for ExportTask {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            // ffmpeg reports progress four times a second; the worker stops at the next report.
            let _ = worker.join();
        }
    }
}

/// Error texts start in lower case (they are also used mid-sentence); a note starts a sentence.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Opens the file manager at `path` with the file selected (where the platform supports that).
pub fn show_in_folder(path: &Path) -> Result<(), String> {
    let spawned = show_in_folder_command(path).spawn();
    spawned
        .map(|_| ())
        .map_err(|err| format!("could not open the folder: {err}"))
}

#[cfg(windows)]
fn show_in_folder_command(path: &Path) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("explorer");
    // Explorer parses its own command line: `/select,"<path>"` must stay one argument. Windows
    // paths cannot contain quotes, so quoting is safe.
    command.raw_arg(format!("/select,\"{}\"", path.display()));
    command
}

#[cfg(target_os = "macos")]
fn show_in_folder_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg("-R").arg(path);
    command
}

#[cfg(not(any(windows, target_os = "macos")))]
fn show_in_folder_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path.parent().unwrap_or(Path::new(".")));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_durations_read_well() {
        assert_eq!(format_size(0), "1 KB");
        assert_eq!(format_size(850_000), "850 KB");
        assert_eq!(format_size(4_400_000), "4.4 MB");
        assert_eq!(format_size(1_234_000_000), "1.23 GB");
        assert_eq!(format_duration(Duration::from_secs(45)), "45 s");
        assert_eq!(format_duration(Duration::from_secs(185)), "3 min 05 s");
        assert_eq!(format_duration(Duration::from_secs(3720)), "1 h 02 min");
    }

    #[test]
    fn missing_encoders_are_replaced() {
        let encoders = Encoders {
            video: vec![VideoCodec::Av1],
            audio: vec![AudioCodec::Opus],
            hardware: Vec::new(),
        };
        let choices = Choices::default().available_in(&encoders);
        assert_eq!(choices.video, VideoCodec::Av1);
        assert_eq!(choices.audio, AudioCodec::Opus);
        assert_eq!(choices.kbps, AudioCodec::Opus.default_bitrate());
        let kept = Choices {
            kbps: 96,
            ..Choices::default()
        };
        assert_eq!(kept.available_in(&Encoders::default()), kept);
    }

    #[test]
    fn choices_survive_a_restart() {
        let mut all = Vec::new();
        for video in VideoCodec::ALL {
            for quality in VideoQuality::ALL {
                for audio in AudioCodec::ALL {
                    for &kbps in audio.bitrates() {
                        for hardware in [false, true] {
                            all.push(Choices {
                                video,
                                quality,
                                audio,
                                kbps,
                                hardware,
                            });
                        }
                    }
                }
            }
        }
        for choices in all {
            assert_eq!(Choices::from_setting(&choices.to_setting()), choices);
        }
        assert_eq!(
            Choices::default().to_setting(),
            "video=h264;quality=high;audio=aac;kbps=192;hardware=0"
        );
    }

    #[test]
    fn broken_settings_fall_back_to_defaults() {
        assert_eq!(Choices::from_setting(""), Choices::default());
        assert_eq!(Choices::from_setting("garbage;;=;x=y"), Choices::default());
        assert_eq!(
            Choices::from_setting("video=vp9;quality=best;kbps=-1;hardware=yes"),
            Choices::default()
        );
        // A bit rate the codec does not offer, or none: the codec's default.
        let opus = Choices::from_setting("audio=opus;kbps=256");
        assert_eq!(
            (opus.audio, opus.kbps),
            (AudioCodec::Opus, AudioCodec::Opus.default_bitrate())
        );
        assert_eq!(Choices::from_setting("audio=opus").kbps, 128);
        // Partial and spaced: what is readable is kept.
        let partial = Choices::from_setting(" video = av1 ; hardware=1 ; future=1");
        assert_eq!(partial.video, VideoCodec::Av1);
        assert!(partial.hardware);
        assert_eq!(partial.quality, VideoQuality::High);
    }

    #[test]
    fn hardware_is_used_only_where_possible() {
        let wanted = Choices {
            hardware: true,
            ..Choices::default()
        };
        let nvenc = [VideoCodec::H264, VideoCodec::H265];
        assert!(wanted.hardware_in_use(&nvenc, false));
        assert!(!wanted.hardware_in_use(&nvenc, true), "rotated");
        assert!(
            !wanted.hardware_in_use(&[VideoCodec::H265], false),
            "no h264_nvenc"
        );
        assert!(
            !Choices::default().hardware_in_use(&nvenc, false),
            "not wanted"
        );
    }

    #[test]
    fn notes_start_with_a_capital() {
        assert_eq!(capitalize("the export failed"), "The export failed");
        assert_eq!(capitalize(""), "");
    }
}
