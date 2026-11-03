//! Playback streams with the installed ffmpeg. Skipped (with a note) if FFmpeg is missing.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use microcut_media::{AudioFormat, FfmpegBackend, MediaBackend, MediaError, VideoFrame};

/// 64×48 gray video whose frame N has luma 16 + 3·N (as in `decode.rs`), no audio.
fn numbered_clip(backend: &FfmpegBackend, fps: u32, ext: &str) -> PathBuf {
    let source =
        format!("color=c=gray:s=64x48:r={fps}:d=1,format=yuv420p,geq=lum='16+3*N':cb=128:cr=128");
    let name = format!("numbered_{fps}fps.{ext}");
    common::generate(
        backend,
        &name,
        &[
            "-f", "lavfi", "-i", &source, "-c:v", "mpeg4", "-q:v", "1", "-g", "12",
        ],
    )
}

/// 2 s of 25 fps video with a 440 Hz stereo tone at 44.1 kHz.
fn clip_with_audio(backend: &FfmpegBackend) -> PathBuf {
    common::generate(
        backend,
        "tone_25fps.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=2",
            "-ac",
            "2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ],
    )
}

fn frame_number(frame: &VideoFrame) -> i64 {
    let (pixels, _) = frame.rgba().as_chunks::<4>();
    let count = pixels.len() as f64;
    let mean_r = pixels.iter().map(|[r, ..]| f64::from(*r)).sum::<f64>() / count;
    let luma = mean_r * 219.0 / 255.0 + 16.0;
    ((luma - 16.0) / 3.0).round() as i64
}

fn stream_numbers(backend: &FfmpegBackend, clip: &Path, from: i64, count: i64) -> Vec<i64> {
    let media = backend.probe(clip).unwrap();
    let mut stream = backend.video_stream(&media, from, count, (64, 48)).unwrap();
    assert_eq!(stream.size(), (64, 48));
    let mut numbers = Vec::new();
    while let Some(frame) = stream.next_frame().unwrap() {
        numbers.push(frame_number(&frame));
    }
    // Stays ended.
    assert!(stream.next_frame().unwrap().is_none());
    numbers
}

#[test]
fn video_stream_delivers_consecutive_exact_frames() {
    let Some(backend) = common::backend() else {
        return;
    };
    let mp4 = numbered_clip(&backend, 25, "mp4");
    assert_eq!(
        stream_numbers(&backend, &mp4, 7, 10),
        (7..17).collect::<Vec<_>>()
    );
    // Starting on a keyframe, and a count beyond the end stops at the last frame.
    assert_eq!(
        stream_numbers(&backend, &mp4, 12, 1000),
        (12..25).collect::<Vec<_>>()
    );
}

#[test]
fn video_stream_is_exact_in_mkv_with_millisecond_timestamps() {
    let Some(backend) = common::backend() else {
        return;
    };
    let mkv = numbered_clip(&backend, 60, "mkv");
    assert_eq!(
        stream_numbers(&backend, &mkv, 13, 1000),
        (13..60).collect::<Vec<_>>()
    );
}

#[test]
fn dropping_a_stream_stops_ffmpeg_promptly() {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = numbered_clip(&backend, 25, "mp4");
    let media = backend.probe(&clip).unwrap();
    let mut stream = backend.video_stream(&media, 0, 25, (64, 48)).unwrap();
    assert_eq!(frame_number(&stream.next_frame().unwrap().unwrap()), 0);
    let start = Instant::now();
    drop(stream);
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn audio_stream_delivers_the_requested_span() {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = clip_with_audio(&backend);
    let media = backend.probe(&clip).unwrap();
    let format = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    // From 0.4 s for 1 s (25 frames at 25 fps), resampled to 48 kHz.
    let mut stream = backend.audio_stream(&media, 10, 25, format).unwrap();
    let mut samples = Vec::new();
    let mut buf = vec![0.0; 4096];
    loop {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        samples.extend_from_slice(&buf[..n]);
    }
    assert_eq!(stream.read(&mut buf).unwrap(), 0);

    let expected = 48_000.0 * 2.0;
    let got = samples.len() as f64;
    assert!((got - expected).abs() < expected * 0.01, "{got} samples");
    let rms = (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / got).sqrt();
    assert!(rms > 0.05, "tone expected, rms {rms}");
}

#[test]
fn audio_stream_rejects_bad_requests() {
    let Some(backend) = common::backend() else {
        return;
    };
    let silent = backend.probe(&numbered_clip(&backend, 25, "mp4")).unwrap();
    let format = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let err = backend.audio_stream(&silent, 0, 25, format).err().unwrap();
    assert!(matches!(err, MediaError::NoAudioStream), "{err:?}");

    let media = backend.probe(&clip_with_audio(&backend)).unwrap();
    let zero = AudioFormat {
        sample_rate: 0,
        channels: 2,
    };
    let err = backend.audio_stream(&media, 0, 25, zero).err().unwrap();
    assert!(matches!(err, MediaError::BadAudioFormat { .. }), "{err:?}");
}
