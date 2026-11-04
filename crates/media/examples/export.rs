//! Exports a frame range without the UI: H.264 "High" and the first audio track as AAC 192k.
//!
//! `cargo run -p microcut-media --example export -- <video> <first frame> <frame count> <output.mp4>`

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use microcut_media::{
    AudioCodec, AudioExport, ExportJob, FfmpegBackend, MediaBackend, VideoCodec, VideoQuality,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, from, count, output] = args.as_slice() else {
        return Err("usage: export <video> <first frame> <frame count> <output.mp4>".to_owned());
    };
    let parse = |text: &str| text.parse::<i64>().map_err(|err| format!("{text}: {err}"));
    let backend = FfmpegBackend::locate().map_err(|err| err.to_string())?;
    let media = backend
        .probe(&PathBuf::from(input))
        .map_err(|err| err.to_string())?;
    let job = ExportJob {
        output: PathBuf::from(output),
        from: parse(from)?,
        count: parse(count)?,
        video: VideoCodec::H264,
        quality: VideoQuality::High,
        hardware: false,
        audio: (!media.audio.is_empty()).then_some(AudioExport {
            track: 0,
            codec: AudioCodec::Aac,
            kbps: 192,
        }),
    };
    let started = Instant::now();
    let end = backend
        .export(&media, &job, &mut |frames| {
            eprint!("\r{frames} / {} frames", job.count);
            true
        })
        .map_err(|err| err.to_string())?;
    eprintln!();
    println!("{end:?} in {:.1} s", started.elapsed().as_secs_f64());
    Ok(())
}
