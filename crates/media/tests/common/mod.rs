//! Shared helpers for integration tests: locating FFmpeg and generating fixtures.

#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    // Write to a temp name first so a half-written file is never reused.
    let tmp = path.with_extension(format!(
        "partial.{}",
        path.extension().unwrap().to_string_lossy()
    ));
    let status = Command::new(backend.ffmpeg_path())
        .args(["-y", "-v", "error"])
        .args(args)
        .arg(&tmp)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg could not create {name}");
    fs::rename(&tmp, &path).unwrap();
    path
}
