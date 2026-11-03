//! Probing real files with the installed ffprobe. Skipped (with a note) if FFmpeg is missing.

// Test-only crate: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use microcut_media::{FfmpegBackend, MediaBackend, MediaError};
use microcut_time::{FrameRate, Tick};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures")
}

fn backend() -> Option<FfmpegBackend> {
    match FfmpegBackend::locate() {
        Ok(backend) => Some(backend),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

/// 2 s, 320×240 at 25 fps with a 48 kHz stereo sine, generated once per test run.
fn sample_clip(backend: &FfmpegBackend) -> &'static Path {
    static CLIP: OnceLock<PathBuf> = OnceLock::new();
    CLIP.get_or_init(|| {
        let dir = fixtures_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("testsrc_320x240_25fps_2s.mp4");
        let status = Command::new(backend.ffmpeg_path())
            .args(["-y", "-v", "error"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=25:duration=2",
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
            ])
            .args([
                "-ac",
                "2",
                "-c:v",
                "mpeg4",
                "-q:v",
                "5",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "ffmpeg could not create the fixture");
        path
    })
}

#[test]
fn probes_generated_clip() {
    let Some(backend) = backend() else { return };
    let info = backend.probe(sample_clip(&backend)).unwrap();

    assert_eq!(info.video.codec, "mpeg4");
    assert_eq!((info.video.width, info.video.height), (320, 240));
    assert_eq!(info.video.frame_rate, FrameRate::new(25, 1).unwrap());
    assert_eq!(info.video.rotation, 0);
    // AAC priming can lengthen the container by a few ms.
    assert!(info.duration >= Tick::from_secs(2), "{:?}", info.duration);
    assert!(info.duration < Tick::parse_decimal_secs("2.1").unwrap());
    assert!(
        (50..=52).contains(&info.frame_count()),
        "{}",
        info.frame_count()
    );
    let audio = info.audio.unwrap();
    assert_eq!(audio.codec, "aac");
    assert_eq!(audio.sample_rate, Some(48_000));
    assert_eq!(audio.channels, Some(2));
    assert!(info.path.is_absolute());
}

#[test]
fn non_media_file_is_reported_not_crashed() {
    let Some(backend) = backend() else { return };
    let dir = fixtures_dir();
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("not-a-video.mp4");
    fs::write(&path, b"this is plain text, not a video").unwrap();

    let err = backend.probe(&path).unwrap_err();
    let text = err.to_string();
    assert!(!text.contains("not-a-video"), "path repeated in: {text}");
    assert!(
        matches!(
            err,
            MediaError::ToolFailed {
                tool: "ffprobe",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn missing_file_is_reported() {
    let Some(backend) = backend() else { return };
    let err = backend.probe(Path::new("no/such/video.mp4")).unwrap_err();
    assert!(matches!(err, MediaError::FileNotFound(_)), "{err:?}");
}

#[test]
fn file_name_starting_with_dash_is_not_an_option() {
    let Some(backend) = backend() else { return };
    let clip = sample_clip(&backend);
    let dashed = clip.with_file_name("-version.mp4");
    fs::copy(clip, &dashed).unwrap();
    // Relative "-version.mp4" would make ffprobe print its version; we pass an absolute path.
    let cwd_relative = Path::new("-version.mp4");
    let info =
        std::env::set_current_dir(dashed.parent().unwrap()).map(|()| backend.probe(cwd_relative));
    assert_eq!(info.unwrap().unwrap().video.width, 320);
}
