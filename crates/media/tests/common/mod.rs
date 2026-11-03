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
