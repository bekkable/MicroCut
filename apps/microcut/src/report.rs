//! What a finished export wrote: shown in the export dialog in place of the settings.

use std::path::Path;
use std::time::Duration;

use eframe::egui;
use microcut_media::{ExportJob, MediaInfo};
use microcut_time::{ClockStamp, FrameRate, TICKS_PER_SECOND, Tick};

use crate::app::format_fps;
use crate::export::{format_duration, format_size};
use crate::tracks;

/// A finished export.
pub struct Report {
    /// What was asked for (`output` is the file written).
    pub job: ExportJob,
    /// How long ffmpeg took.
    pub elapsed: Duration,
    /// Size of the file, if it could be read.
    pub bytes: Option<u64>,
    /// The written file as probed afterwards, if that worked.
    pub written: Option<MediaInfo>,
}

impl Report {
    /// Frames in the file (as probed, else as asked for).
    fn frames(&self) -> i64 {
        self.written
            .as_ref()
            .map_or(self.job.count, MediaInfo::frame_count)
    }

    /// Length of the file (as probed, else from the frames and `rate`).
    fn duration(&self, rate: FrameRate) -> Tick {
        match &self.written {
            Some(info) => info.duration,
            None => rate.frame_to_tick(self.job.count),
        }
    }
}

/// File name, then a grid of facts about the file and the encoding.
pub fn show(ui: &mut egui::Ui, report: &Report, source: &MediaInfo) {
    let output = &report.job.output;
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
    let dir = output.parent().unwrap_or(Path::new(""));
    ui.add(
        egui::Label::new(egui::RichText::new(format!("in {}", dir.display())).weak()).truncate(),
    );
    ui.add_space(8.0);

    let rate = source.video.frame_rate;
    let duration = report.duration(rate);
    let video = report.written.as_ref().map_or(&source.video, |w| &w.video);
    let (width, height) = video.display_size();
    egui::Grid::new("export report")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("File");
            ui.label(match report.bytes {
                Some(bytes) => format!(
                    "{} · {}",
                    format_size(bytes),
                    format_bitrate(bytes, duration)
                ),
                None => "size unknown".to_owned(),
            });
            ui.end_row();

            ui.label("Content");
            ui.label(format!(
                "{} frames · {} · {width}×{height} · {}",
                report.frames(),
                ClockStamp(duration),
                format_fps(rate)
            ));
            ui.end_row();

            ui.label("Video");
            ui.label(video_settings(&report.job));
            ui.end_row();

            ui.label("Audio");
            ui.label(audio_settings(&report.job, source));
            ui.end_row();

            ui.label("Encoding");
            ui.label(encoding_speed(report.frames(), duration, report.elapsed));
            ui.end_row();
        });
}

/// `H.264 · High (CRF 18) · libx264`, `H.265 · High (CQ 28) · NVIDIA hevc_nvenc`
fn video_settings(job: &ExportJob) -> String {
    let codec = job.video;
    let quality = job.quality.label();
    if job.hardware {
        format!(
            "{} · {quality} (CQ {}) · NVIDIA {}",
            codec.label(),
            codec.hardware_cq(job.quality),
            codec.hardware_encoder()
        )
    } else {
        format!(
            "{} · {quality} (CRF {}) · {}",
            codec.label(),
            codec.crf(job.quality),
            codec.encoder()
        )
    }
}

/// `AAC 192 kbit/s · Audio track 1: Music · … · sync +150 ms`, or `None`.
fn audio_settings(job: &ExportJob, source: &MediaInfo) -> String {
    let Some(audio) = &job.audio else {
        return "None".to_owned();
    };
    let mut text = format!("{} {} kbit/s", audio.codec.label(), audio.kbps);
    if let Some(info) = source.audio.get(audio.track) {
        text.push_str(&format!(" · {}", tracks::tooltip(audio.track, info)));
    }
    if audio.delay != Tick::ZERO {
        let ms = audio.delay.0 / (TICKS_PER_SECOND / 1000);
        text.push_str(&format!(" · sync {ms:+} ms"));
    }
    text
}

/// Average bit rate of the whole file: `5.6 Mbit/s`, `850 kbit/s`.
fn format_bitrate(bytes: u64, duration: Tick) -> String {
    let secs = duration.0 as f64 / TICKS_PER_SECOND as f64;
    if secs.is_nan() || secs <= 0.0 {
        return "bit rate unknown".to_owned();
    }
    let bps = bytes as f64 * 8.0 / secs;
    if bps < 1e6 {
        format!("{:.0} kbit/s", bps / 1e3)
    } else {
        format!("{:.1} Mbit/s", bps / 1e6)
    }
}

/// `12.3 s · 309 fps · 5.2× real time`
fn encoding_speed(frames: i64, duration: Tick, elapsed: Duration) -> String {
    let took = elapsed.as_secs_f64();
    let time = if took < 60.0 {
        format!("{took:.1} s")
    } else {
        format_duration(elapsed)
    };
    if took <= 0.0 {
        return time;
    }
    let fps = frames.max(0) as f64 / took;
    let real_time = duration.0 as f64 / TICKS_PER_SECOND as f64 / took;
    format!("{time} · {fps:.0} fps · {real_time:.1}× real time")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use microcut_media::{AudioCodec, AudioExport, AudioInfo, VideoCodec, VideoInfo, VideoQuality};

    use super::*;

    fn job(hardware: bool, audio: Option<AudioExport>) -> ExportJob {
        ExportJob {
            output: PathBuf::from("/out/cut.mp4"),
            from: 0,
            count: 600,
            video: VideoCodec::H264,
            quality: VideoQuality::High,
            hardware,
            audio,
        }
    }

    fn source() -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("/videos/clip.mkv"),
            container: "matroska".to_owned(),
            start: Tick::ZERO,
            duration: Tick::from_secs(60),
            video: VideoInfo {
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(60, 1).unwrap(),
                rotation: 0,
            },
            audio: vec![AudioInfo {
                codec: "aac".to_owned(),
                sample_rate: Some(48_000),
                channels: Some(2),
                title: Some("Music".to_owned()),
                language: None,
            }],
        }
    }

    #[test]
    fn settings_read_well() {
        assert_eq!(
            video_settings(&job(false, None)),
            "H.264 · High (CRF 18) · libx264"
        );
        assert_eq!(
            video_settings(&job(true, None)),
            "H.264 · High (CQ 27) · NVIDIA h264_nvenc"
        );
        assert_eq!(audio_settings(&job(false, None), &source()), "None");
        let aac = AudioExport {
            track: 0,
            codec: AudioCodec::Aac,
            kbps: 192,
            delay: Tick::from_millis(-150),
        };
        assert_eq!(
            audio_settings(&job(false, Some(aac)), &source()),
            "AAC 192 kbit/s · Audio track 0: Music · aac 48 kHz stereo · sync -150 ms"
        );
    }

    #[test]
    fn rates_and_speed() {
        let ten_secs = Tick::from_secs(10);
        assert_eq!(format_bitrate(7_000_000, ten_secs), "5.6 Mbit/s");
        assert_eq!(format_bitrate(1_000_000, ten_secs), "800 kbit/s");
        assert_eq!(format_bitrate(1_000, Tick::ZERO), "bit rate unknown");
        assert_eq!(
            encoding_speed(600, ten_secs, Duration::from_millis(2000)),
            "2.0 s · 300 fps · 5.0× real time"
        );
        assert_eq!(
            encoding_speed(600, ten_secs, Duration::from_secs(125)),
            "2 min 05 s · 5 fps · 0.1× real time"
        );
        assert_eq!(encoding_speed(600, ten_secs, Duration::ZERO), "0.0 s");
    }
}
