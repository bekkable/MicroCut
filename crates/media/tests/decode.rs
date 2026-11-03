//! Frame-accurate decoding with the installed ffmpeg. Skipped (with a note) if FFmpeg is missing.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use microcut_media::{FfmpegBackend, MediaBackend, MediaError, VideoFrame};

/// 64×48 gray video whose frame N has luma 16 + 3·N, so every frame identifies itself.
/// Keyframe every 12 frames, so most seeks have to decode forward from a keyframe.
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

/// Frame number encoded in a decoded picture (inverse of the limited-range YUV→RGB conversion).
fn frame_number(frame: &VideoFrame) -> i64 {
    let (pixels, _) = frame.rgba().as_chunks::<4>();
    let count = pixels.len() as f64;
    let mean_r = pixels.iter().map(|[r, ..]| f64::from(*r)).sum::<f64>() / count;
    let luma = mean_r * 219.0 / 255.0 + 16.0;
    ((luma - 16.0) / 3.0).round() as i64
}

fn assert_frames_exact(fps: u32, ext: &str) {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = numbered_clip(&backend, fps, ext);
    let media = backend.probe(&clip).unwrap();
    assert_eq!(media.frame_count(), i64::from(fps), "{ext}");

    let last = i64::from(fps) - 1;
    for wanted in [0, 1, 2, 11, 12, 13, 23, 24, last - 1, last] {
        let frame = backend.decode_frame(&media, wanted, (64, 48)).unwrap();
        assert_eq!(frame_number(&frame), wanted, "{fps} fps {ext}");
    }
}

#[test]
fn every_requested_frame_is_exact_in_mp4() {
    assert_frames_exact(25, "mp4");
    assert_frames_exact(60, "mp4");
}

#[test]
fn every_requested_frame_is_exact_in_mkv_with_millisecond_timestamps() {
    assert_frames_exact(60, "mkv");
    assert_frames_exact(30, "mkv");
}

#[test]
fn frame_is_scaled_to_fit_and_never_upscaled() {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = numbered_clip(&backend, 25, "mp4");
    let media = backend.probe(&clip).unwrap();

    let small = backend.decode_frame(&media, 5, (32, 100)).unwrap();
    assert_eq!((small.width(), small.height()), (32, 24));
    assert_eq!(small.rgba().len(), 32 * 24 * 4);

    let big = backend.decode_frame(&media, 5, (5000, 5000)).unwrap();
    assert_eq!((big.width(), big.height()), (64, 48));
}

#[test]
fn out_of_range_frames_are_clamped() {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = numbered_clip(&backend, 25, "mp4");
    let media = backend.probe(&clip).unwrap();

    let before = backend.decode_frame(&media, -100, (64, 48)).unwrap();
    assert_eq!(frame_number(&before), 0);
    let after = backend.decode_frame(&media, i64::MAX, (64, 48)).unwrap();
    assert_eq!(frame_number(&after), 24);
}

#[test]
fn deleted_file_is_an_error() {
    let Some(backend) = common::backend() else {
        return;
    };
    let clip = numbered_clip(&backend, 25, "mp4");
    let mut media = backend.probe(&clip).unwrap();
    media.path = common::fixtures_dir().join("gone.mp4");

    let err = backend.decode_frame(&media, 0, (64, 48)).unwrap_err();
    assert!(
        matches!(err, MediaError::ToolFailed { tool: "ffmpeg", .. }),
        "{err:?}"
    );
}
