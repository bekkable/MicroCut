//! Prints what MicroCut reads from video files, without the UI, and times decoding the first,
//! middle and last frame.
//!
//! `cargo run -p microcut-media --example probe -- <file>…`

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use microcut_media::{FfmpegBackend, MediaBackend};
use microcut_time::ClockStamp;

fn main() -> ExitCode {
    let backend = match FfmpegBackend::locate() {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let version = backend
        .version()
        .map_or_else(|| "development build".to_owned(), |v| v.to_string());
    println!(
        "ffmpeg {} ({version}), ffprobe {}",
        backend.ffmpeg_path().display(),
        backend.ffprobe_path().display()
    );
    let mut failed = false;
    for arg in std::env::args_os().skip(1) {
        let path = Path::new(&arg);
        println!("{}", path.display());
        match backend.probe(path) {
            Ok(info) => {
                let v = &info.video;
                println!("  container {}", info.container);
                println!(
                    "  start     {}  duration {}  frames {}",
                    ClockStamp(info.start),
                    ClockStamp(info.duration),
                    info.frame_count()
                );
                println!(
                    "  video     {} {}x{} @ {}/{} rotation {}",
                    v.codec,
                    v.width,
                    v.height,
                    v.frame_rate.num(),
                    v.frame_rate.den(),
                    v.rotation
                );
                if info.audio.is_empty() {
                    println!("  audio     none");
                }
                for (track, a) in info.audio.iter().enumerate() {
                    println!(
                        "  audio {track}   {} {:?} Hz {:?} ch, title {:?}, language {:?}",
                        a.codec, a.sample_rate, a.channels, a.title, a.language
                    );
                }
                let last = info.frame_count().saturating_sub(1);
                for frame in [0, last / 2, last] {
                    let started = Instant::now();
                    let result = backend.decode_frame(&info, frame, (960, 540));
                    let ms = started.elapsed().as_millis();
                    match result {
                        Ok(f) => {
                            println!("  frame {frame:>7} {}x{} in {ms} ms", f.width(), f.height())
                        }
                        Err(err) => println!("  frame {frame:>7} ERROR {err} ({ms} ms)"),
                    }
                }
            }
            Err(err) => {
                failed = true;
                println!("  ERROR {err}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
