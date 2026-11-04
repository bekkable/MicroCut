//! Export choices that do not depend on the UI: the file name, the preselected audio track and
//! the size estimate.

use std::path::Path;

use microcut_media::{AudioInfo, MediaInfo, VideoCodec, VideoQuality};
use microcut_time::range_label;

use crate::FrameRange;

/// Extension of every export.
pub const EXPORT_EXTENSION: &str = "mp4";

/// Longest file name accepted (characters), well below the limits of common file systems.
const MAX_NAME_CHARS: usize = 200;

/// `<stem>_hh-mm-ss-to-hh-mm-ss.mp4` for `range` of `media`.
pub fn default_file_name(media: &MediaInfo, range: FrameRange) -> String {
    let rate = media.video.frame_rate;
    let stem = media
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".to_owned());
    let label = range_label(range.start_time(rate), range.end_time(rate));
    format!("{stem}_{label}.{EXPORT_EXTENSION}")
}

/// Checks a file name typed by the user and adds `.mp4` unless it ends with it. The error is a
/// sentence for the user.
pub fn clean_file_name(typed: &str) -> Result<String, &'static str> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("Enter a file name.");
    }
    if name
        .chars()
        .any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c))
    {
        return Err(r#"A file name cannot contain \ / : * ? " < > |."#);
    }
    let name = if has_extension(name) {
        name.to_owned()
    } else {
        format!("{}.{EXPORT_EXTENSION}", name.trim_end_matches(['.', ' ']))
    };
    let stem = name
        .get(..name.len().saturating_sub(EXPORT_EXTENSION.len() + 1))
        .unwrap_or_default();
    if stem.trim_matches(['.', ' ']).is_empty() {
        return Err("Enter a file name.");
    }
    if stem.ends_with(['.', ' ']) {
        return Err("A file name cannot end with a dot or a space.");
    }
    if is_reserved(stem) {
        return Err("This name is reserved by Windows. Choose another one.");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err("The file name is too long.");
    }
    Ok(name)
}

fn has_extension(name: &str) -> bool {
    let tail = name.len().checked_sub(EXPORT_EXTENSION.len() + 1);
    tail.and_then(|at| name.get(at..))
        .is_some_and(|ext| ext.eq_ignore_ascii_case(&format!(".{EXPORT_EXTENSION}")))
}

/// `CON`, `NUL`, `COM1`, … (also with an extension) cannot be file names on Windows.
fn is_reserved(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit() && c != '0'))
        })
}

/// `name` if no such file exists in `dir`, else `name (2).mp4`, `name (3).mp4`, …
/// `exists` is asked for each candidate (the file system, or a fake in tests). `None` if even
/// the 999th candidate is taken.
pub fn free_file_name(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> Option<String> {
    if !exists(&dir.join(name)) {
        return Some(name.to_owned());
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (2..1000)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !exists(&dir.join(candidate)))
}

/// The audio track an export starts with: one titled "Music" (any case), else the first.
pub fn default_audio_track(audio: &[AudioInfo]) -> Option<usize> {
    let music = audio.iter().position(|a| {
        a.title
            .as_deref()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("music"))
    });
    music.or((!audio.is_empty()).then_some(0))
}

/// Rough size in bytes of an export of `frames` frames: a bits-per-pixel model calibrated on
/// screen and game recordings (real sizes vary a lot with the content), plus the audio bit rate.
pub fn estimate_bytes(
    media: &MediaInfo,
    frames: i64,
    (video, quality, hardware): (VideoCodec, VideoQuality, bool),
    audio_kbps: Option<u32>,
) -> u64 {
    let rate = media.video.frame_rate;
    let fps = f64::from(rate.num()) / f64::from(rate.den().max(1));
    let secs = frames.max(0) as f64 / fps.max(1e-3);
    let (w, h) = media.video.display_size();
    let pixels = f64::from(w) * f64::from(h);
    // Measured on 720p60 recordings (one menu, three gameplay scenes): x264 "High" averages
    // 5.5 Mbit/s, "Medium" 54 % and "Small" 30 % of that; x265 needs 72 %. NVENC at the
    // calibrated CQ values (`VideoCodec::hardware_cq`) is about as large as x264 and 7 % larger
    // than x265. AV1 is assumed a bit smaller than H.265.
    let codec = match video {
        VideoCodec::H264 => 1.0,
        VideoCodec::H265 => 0.72,
        VideoCodec::Av1 => 0.6,
    };
    let level = match quality {
        VideoQuality::High => 1.0,
        VideoQuality::Medium => 0.54,
        VideoQuality::Small => 0.3,
    };
    let hardware = match (hardware, video) {
        (true, VideoCodec::H265 | VideoCodec::Av1) => 1.07,
        _ => 1.0,
    };
    // Later frames repeat much of earlier ones: bits grow slower than the frame rate.
    let video_bps = 0.34 * pixels * fps.max(1.0).powf(0.7) * codec * level * hardware;
    let audio_bps = f64::from(audio_kbps.unwrap_or(0)) * 1000.0;
    let bytes = (video_bps + audio_bps) * secs / 8.0;
    if bytes.is_finite() && bytes > 0.0 {
        bytes as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use microcut_media::VideoInfo;
    use microcut_time::{FrameRate, Tick};

    use super::*;
    use crate::Session;

    fn media(fps: u32, secs: i64) -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("/videos/My clip.final.mkv"),
            container: "matroska".to_owned(),
            start: Tick::ZERO,
            duration: Tick::from_secs(secs),
            video: VideoInfo {
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: FrameRate::new(fps, 1).unwrap(),
                rotation: 0,
            },
            audio: Vec::new(),
        }
    }

    fn track(title: Option<&str>) -> AudioInfo {
        AudioInfo {
            codec: "aac".to_owned(),
            sample_rate: Some(48_000),
            channels: Some(2),
            title: title.map(str::to_owned),
            language: None,
        }
    }

    #[test]
    fn default_name_carries_the_range() {
        let mut session = Session::new(Arc::new(media(60, 3600)));
        session.set_range_start(60 * 65);
        session.set_range_end(60 * 130 - 1);
        assert_eq!(
            default_file_name(session.media(), session.range()),
            "My clip.final_00-01-05-to-00-02-10.mp4"
        );
    }

    #[test]
    fn typed_names_are_checked() {
        assert_eq!(clean_file_name("  cut "), Ok("cut.mp4".to_owned()));
        assert_eq!(clean_file_name("cut.MP4"), Ok("cut.MP4".to_owned()));
        assert_eq!(clean_file_name("cut.mkv"), Ok("cut.mkv.mp4".to_owned()));
        assert_eq!(clean_file_name("cut."), Ok("cut.mp4".to_owned()));
        assert_eq!(clean_file_name("a (2)"), Ok("a (2).mp4".to_owned()));
        assert!(clean_file_name("").is_err());
        assert!(clean_file_name(".mp4").is_err());
        assert!(clean_file_name("...").is_err());
        assert!(clean_file_name("a/b").is_err());
        assert!(clean_file_name("..\\up").is_err());
        assert!(clean_file_name("c:x").is_err());
        assert!(clean_file_name("tab\there").is_err());
        assert!(clean_file_name("cut ..mp4").is_err());
        assert!(clean_file_name("nul").is_err());
        assert!(clean_file_name("Com3.mp4").is_err());
        assert!(clean_file_name("com0").is_ok());
        assert!(clean_file_name("console").is_ok());
        assert!(clean_file_name(&"x".repeat(300)).is_err());
        assert!(clean_file_name("äöü 日本").is_ok());
    }

    #[test]
    fn taken_names_get_a_number() {
        let dir = Path::new("/out");
        let taken = ["/out/cut.mp4", "/out/cut (2).mp4"].map(PathBuf::from);
        let exists = |p: &Path| taken.iter().any(|t| t == p);
        assert_eq!(
            free_file_name(dir, "new.mp4", exists),
            Some("new.mp4".to_owned())
        );
        assert_eq!(
            free_file_name(dir, "cut.mp4", exists),
            Some("cut (3).mp4".to_owned())
        );
        assert_eq!(free_file_name(dir, "x.mp4", |_| true), None);
    }

    #[test]
    fn music_is_preselected() {
        assert_eq!(default_audio_track(&[]), None);
        assert_eq!(default_audio_track(&[track(None)]), Some(0));
        let obs = [track(Some("Mic")), track(Some(" music "))];
        assert_eq!(default_audio_track(&obs), Some(1));
        let other = [track(Some("Mic")), track(Some("Game"))];
        assert_eq!(default_audio_track(&other), Some(0));
    }

    #[test]
    fn estimates_follow_the_settings() {
        let media = media(60, 600);
        let high = estimate_bytes(
            &media,
            600,
            (VideoCodec::H264, VideoQuality::High, false),
            None,
        );
        // 10 s of 720p60 H.264 "High": measured 3.9 to 9.6 MB.
        assert!((5_000_000..8_000_000).contains(&high), "{high}");
        let small = estimate_bytes(
            &media,
            600,
            (VideoCodec::H264, VideoQuality::Small, false),
            None,
        );
        let av1 = estimate_bytes(
            &media,
            600,
            (VideoCodec::Av1, VideoQuality::High, false),
            None,
        );
        assert!(small < high && av1 < high);
        let nvenc = estimate_bytes(
            &media,
            600,
            (VideoCodec::H264, VideoQuality::High, true),
            None,
        );
        assert_eq!(nvenc, high, "NVENC is calibrated to x264's sizes");
        let hevc = |hardware| {
            estimate_bytes(
                &media,
                600,
                (VideoCodec::H265, VideoQuality::High, hardware),
                None,
            )
        };
        assert!(
            hevc(true) > hevc(false),
            "hevc_nvenc is a little larger than x265"
        );
        let with_audio = estimate_bytes(
            &media,
            600,
            (VideoCodec::H264, VideoQuality::High, false),
            Some(192),
        );
        assert_eq!(with_audio - high, 240_000, "192 kbit/s for 10 s");
        assert_eq!(
            estimate_bytes(
                &media,
                -5,
                (VideoCodec::H264, VideoQuality::High, false),
                None
            ),
            0
        );
    }
}
