//! Shared helpers for integration tests: locating FFmpeg and generating fixtures.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use microcut_media::FfmpegBackend;

pub fn fixtures_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The backend, or `None` (with a note) when FFmpeg is not installed.
pub fn backend() -> Option<FfmpegBackend> {
    match FfmpegBackend::locate() {
        Ok(backend) => Some(backend),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

/// Runs ffmpeg with `args` to create `fixtures/<name>` unless it exists; returns its path.
pub fn generate(backend: &FfmpegBackend, name: &str, args: &[&str]) -> PathBuf {
    let path = fixtures_dir().join(name);
    if path.is_file() {
        return path;
    }
    // Write to a temp name first so a half-written file is never reused. Tests run in parallel
    // and may want the same fixture: each writes its own temp file, the first rename wins.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let tmp = path.with_extension(format!(
        "partial-{}-{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        path.extension().unwrap().to_string_lossy()
    ));
    let status = Command::new(backend.ffmpeg_path())
        .args(["-y", "-v", "error"])
        .args(args)
        .arg(&tmp)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create {name}");
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
        assert!(path.is_file(), "could not store fixture {name}");
    }
    path
}

/// 2 s of 25 fps video whose sound (48 kHz stereo PCM) is silent for exactly 1 s, then a 440 Hz
/// tone: where the tone starts tells where the sound was taken from.
pub fn tone_after_one_second(backend: &FfmpegBackend) -> PathBuf {
    generate(
        backend,
        "tone_after_1s_25fps.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            r"aevalsrc=exprs=if(gte(t\,1)\,0.5*sin(2*PI*440*t)\,0):s=48000:c=stereo:d=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "pcm_s16le",
        ],
    )
}

/// Seconds into interleaved stereo `samples` (48 kHz) until the first loud sample; `-1` if
/// there is none.
pub fn onset_secs(samples: &[f32]) -> f64 {
    match samples.iter().position(|s| s.abs() > 0.05) {
        Some(first) => first as f64 / 2.0 / 48_000.0,
        None => -1.0,
    }
}
