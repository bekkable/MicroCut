//! Export with the installed ffmpeg. Skipped (with a note) if FFmpeg or an encoder is missing.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use microcut_media::{
    AudioCodec, AudioExport, AudioFormat, ExportEnd, ExportJob, FfmpegBackend, MediaBackend,
    MediaError, MediaInfo, VideoCodec, VideoFrame, VideoQuality,
};

/// Gray video of `w`×`h` whose frame N has luma 16 + 3·N (as in `stream.rs`), no audio.
fn numbered_clip(backend: &FfmpegBackend, fps: u32, (w, h): (u32, u32), ext: &str) -> PathBuf {
    let source =
        format!("color=c=gray:s={w}x{h}:r={fps}:d=1,format=yuv420p,geq=lum='16+3*N':cb=128:cr=128");
    let name = format!("numbered_{fps}fps_{w}x{h}.{ext}");
    let args = [
        "-f", "lavfi", "-i", &source, "-c:v", "mpeg4", "-q:v", "1", "-g", "12",
    ];
    common::generate(backend, &name, &args)
}

/// 2 s of 25 fps video with two titled audio tracks: 0 = "Tone" (440 Hz), 1 = "Silence".
fn clip_with_two_tracks(backend: &FfmpegBackend) -> PathBuf {
    common::generate(
        backend,
        "two_tracks_25fps.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x48:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-map",
            "0",
            "-map",
            "1",
            "-map",
            "2",
            "-metadata:s:a:0",
            "title=Tone",
            "-metadata:s:a:1",
            "title=Silence",
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

/// A fresh, empty directory for the outputs of one test.
fn out_dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("exports")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The backend if it can encode `video` (and AAC), else `None` with a note.
fn backend_with(video: VideoCodec) -> Option<FfmpegBackend> {
    let backend = common::backend()?;
    let encoders = backend.encoders().unwrap();
    if !encoders.video.contains(&video) || !encoders.audio.contains(&AudioCodec::Aac) {
        eprintln!("skipping: FFmpeg has no {} encoder", video.encoder());
        return None;
    }
    Some(backend)
}

fn job(output: PathBuf, from: i64, count: i64) -> ExportJob {
    ExportJob {
        output,
        from,
        count,
        video: VideoCodec::H264,
        quality: VideoQuality::High,
        hardware: false,
        audio: None,
    }
}

fn export(backend: &FfmpegBackend, media: &MediaInfo, job: &ExportJob) -> Vec<i64> {
    let mut reports = Vec::new();
    let end = backend
        .export(media, job, &mut |frames| {
            reports.push(frames);
            true
        })
        .unwrap();
    assert_eq!(end, ExportEnd::Finished);
    reports
}

/// The frame numbers of every picture of `path`.
fn numbers(backend: &FfmpegBackend, path: &Path) -> Vec<i64> {
    let media = backend.probe(path).unwrap();
    let mut stream = backend.video_stream(&media, 0, 1000, (64, 48)).unwrap();
    let mut numbers = Vec::new();
    while let Some(frame) = stream.next_frame().unwrap() {
        numbers.push(frame_number(&frame));
    }
    numbers
}

#[test]
fn export_contains_exactly_the_range() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let dir = out_dir("exact");
    for (fps, ext) in [(25, "mp4"), (60, "mkv")] {
        let clip = numbered_clip(&backend, fps, (64, 48), ext);
        let media = backend.probe(&clip).unwrap();
        let output = dir.join(format!("cut_{fps}.mp4"));
        let reports = export(&backend, &media, &job(output.clone(), 7, 10));
        assert!(!reports.is_empty(), "progress was reported");
        assert_eq!(reports.last(), Some(&10), "{reports:?}");

        let expected: Vec<i64> = (7..17).collect();
        assert_eq!(numbers(&backend, &output), expected, "{fps} fps {ext}");
        let out = backend.probe(&output).unwrap();
        assert_eq!(out.video.frame_rate, media.video.frame_rate);
        assert!(out.audio.is_empty());
    }
}

/// CUDA decoding and NVENC: as exact as the software path. Skipped without an NVENC encoder;
/// with one but no usable NVIDIA card the export fails, which is reported as a skip too.
#[test]
fn hardware_export_contains_exactly_the_range() {
    let Some(backend) = common::backend() else {
        return;
    };
    let encoders = backend.encoders().unwrap();
    let dir = out_dir("hardware");
    for codec in encoders.hardware {
        // NVENC refuses very small pictures.
        let clip = numbered_clip(&backend, 25, (256, 144), "mp4");
        let media = backend.probe(&clip).unwrap();
        let output = dir.join(format!("{}.mp4", codec.hardware_encoder()));
        let job = ExportJob {
            video: codec,
            hardware: true,
            ..job(output.clone(), 7, 10)
        };
        match backend.export(&media, &job, &mut |_| true) {
            Ok(end) => assert_eq!(end, ExportEnd::Finished),
            Err(MediaError::ExportFailed { message }) => {
                eprintln!("skipping {}: {message}", codec.hardware_encoder());
                continue;
            }
            Err(err) => panic!("{err:?}"),
        }
        let expected: Vec<i64> = (7..17).collect();
        assert_eq!(numbers(&backend, &output), expected, "{codec:?}");
        let out = backend.probe(&output).unwrap();
        assert_eq!((out.video.width, out.video.height), (256, 144));
    }
}

#[test]
fn odd_sizes_are_cropped_to_even() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let media = backend
        .probe(&numbered_clip(&backend, 25, (65, 49), "mp4"))
        .unwrap();
    let output = out_dir("odd").join("cut.mp4");
    export(&backend, &media, &job(output.clone(), 0, 5));
    let out = backend.probe(&output).unwrap();
    assert_eq!((out.video.width, out.video.height), (64, 48));
}

#[test]
fn every_available_codec_exports() {
    let Some(backend) = common::backend() else {
        return;
    };
    let encoders = backend.encoders().unwrap();
    // SVT-AV1 needs at least 64×64 pixels.
    let clip = common::generate(
        &backend,
        "tone_160x96_25fps.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=160x96:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let media = backend.probe(&clip).unwrap();
    let dir = out_dir("codecs");
    for video in encoders.video {
        for audio in &encoders.audio {
            let output = dir.join(format!("{}_{}.mp4", video.encoder(), audio.encoder()));
            let job = ExportJob {
                video,
                quality: VideoQuality::Small,
                audio: Some(AudioExport {
                    track: 0,
                    codec: *audio,
                    kbps: audio.default_bitrate(),
                }),
                ..job(output.clone(), 5, 20)
            };
            export(&backend, &media, &job);
            let out = backend.probe(&output).unwrap();
            assert_eq!(out.frame_count(), 20, "{output:?}");
            assert_eq!(out.audio.len(), 1, "{output:?}");
        }
    }
}

#[test]
fn the_chosen_audio_track_is_exported() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let media = backend.probe(&clip_with_two_tracks(&backend)).unwrap();
    let dir = out_dir("tracks");
    let mut levels = Vec::new();
    for track in [0, 1] {
        let output = dir.join(format!("track_{track}.mp4"));
        let job = ExportJob {
            audio: Some(AudioExport {
                track,
                codec: AudioCodec::Aac,
                kbps: 128,
            }),
            ..job(output.clone(), 10, 25)
        };
        export(&backend, &media, &job);
        let out = backend.probe(&output).unwrap();
        assert_eq!(out.audio.len(), 1);
        let samples = read_audio(&backend, &out);
        // One second at 48 kHz stereo, give or take the encoder's padding.
        let expected = 96_000.0;
        let got = samples.len() as f64;
        assert!((got - expected).abs() < 4096.0, "{got} samples");
        levels.push(rms(&samples));
    }
    assert!(levels[0] > 0.05, "track 0 is the tone: {levels:?}");
    assert!(levels[1] < 1e-3, "track 1 is silent: {levels:?}");
}

fn read_audio(backend: &FfmpegBackend, media: &MediaInfo) -> Vec<f32> {
    let format = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let count = media.frame_count();
    let mut stream = backend.audio_stream(media, 0, 0, count, format).unwrap();
    let mut samples = Vec::new();
    let mut buf = vec![0.0; 4096];
    loop {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        samples.extend_from_slice(buf.get(..n).unwrap());
    }
    samples
}

fn rms(samples: &[f32]) -> f64 {
    let n = samples.len().max(1) as f64;
    (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / n).sqrt()
}

#[test]
fn existing_files_are_never_overwritten() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let media = backend
        .probe(&numbered_clip(&backend, 25, (64, 48), "mp4"))
        .unwrap();
    let output = out_dir("exists").join("cut.mp4");
    fs::write(&output, b"precious").unwrap();
    let err = backend
        .export(&media, &job(output.clone(), 0, 5), &mut |_| true)
        .unwrap_err();
    assert!(matches!(err, MediaError::OutputExists(_)), "{err:?}");
    assert_eq!(fs::read(&output).unwrap(), b"precious");
}

#[test]
fn cancelling_removes_the_partial_file() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let media = backend
        .probe(&numbered_clip(&backend, 25, (64, 48), "mp4"))
        .unwrap();
    let output = out_dir("cancel").join("cut.mp4");
    let end = backend
        .export(&media, &job(output.clone(), 0, 25), &mut |_| false)
        .unwrap();
    assert_eq!(end, ExportEnd::Cancelled);
    assert!(!output.exists());
}

#[test]
fn failures_are_reported_and_leave_no_file() {
    let Some(backend) = backend_with(VideoCodec::H264) else {
        return;
    };
    let dir = out_dir("fail");
    // A source that disappears after probing: ffmpeg cannot open it.
    let source = dir.join("gone.mp4");
    fs::copy(numbered_clip(&backend, 25, (64, 48), "mp4"), &source).unwrap();
    let media = backend.probe(&source).unwrap();
    fs::remove_file(&source).unwrap();
    let output = dir.join("cut.mp4");
    let err = backend
        .export(&media, &job(output.clone(), 0, 5), &mut |_| true)
        .unwrap_err();
    assert!(matches!(err, MediaError::ExportFailed { .. }), "{err:?}");
    assert!(!output.exists());

    // Bad requests are refused before anything is created.
    let media = backend
        .probe(&numbered_clip(&backend, 25, (64, 48), "mp4"))
        .unwrap();
    let empty = backend.export(&media, &job(output.clone(), 0, 0), &mut |_| true);
    assert!(matches!(empty, Err(MediaError::BadExport(_))), "{empty:?}");
    let no_track = ExportJob {
        audio: Some(AudioExport {
            track: 0,
            codec: AudioCodec::Aac,
            kbps: 128,
        }),
        ..job(output.clone(), 0, 5)
    };
    let err = backend.export(&media, &no_track, &mut |_| true);
    assert!(matches!(err, Err(MediaError::NoAudioStream)), "{err:?}");
    assert!(!output.exists());
}
