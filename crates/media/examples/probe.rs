//! Prints what MicroCut reads from video files, without the UI.
//!
//! `cargo run -p microcut-media --example probe -- <file>…`

use std::path::Path;
use std::process::ExitCode;

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
                match &info.audio {
                    Some(a) => println!(
                        "  audio     {} {:?} Hz {:?} ch",
                        a.codec, a.sample_rate, a.channels
                    ),
                    None => println!("  audio     none"),
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
