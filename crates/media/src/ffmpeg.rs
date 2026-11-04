//! [`MediaBackend`] implementation that runs `ffmpeg`/`ffprobe` as separate processes.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use microcut_time::{DecimalSecs, TICKS_PER_SECOND, Tick};

use crate::export::ProgressParser;
use crate::frame::{self, fit_within, rgba_len};
use crate::locate::{find_tool, search_dirs};
use crate::process::{self, Limits, Streaming};
use crate::stream::{self, FfmpegAudioStream, FfmpegVideoStream};
use crate::version::FfmpegVersion;
use crate::{
    AudioFormat, AudioStream, Encoders, ExportEnd, ExportJob, MediaBackend, MediaError, MediaInfo,
    VideoCodec, VideoFrame, VideoStream, probe,
};

const PROBE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(30),
    max_stdout: 16 * 1024 * 1024,
    max_stderr: 64 * 1024,
};

/// Seeking decodes from the previous keyframe, which can be seconds of video.
const FRAME_TIMEOUT: Duration = Duration::from_secs(20);

const VERSION_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(15),
    max_stdout: 64 * 1024,
    max_stderr: 64 * 1024,
};

/// Media backend using the `ffmpeg` and `ffprobe` programs.
#[derive(Debug, Clone)]
pub struct FfmpegBackend {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    /// Read on first use (so finding the programs never waits for one to run).
    version: OnceLock<Option<FfmpegVersion>>,
}

impl FfmpegBackend {
    /// Finds `ffmpeg` and `ffprobe` (see [`crate::FFMPEG_DIR_ENV`] for the search order).
    pub fn locate() -> Result<FfmpegBackend, MediaError> {
        let dirs = search_dirs();
        let find =
            |tool: &'static str| find_tool(tool, &dirs).ok_or(MediaError::ToolNotFound { tool });
        Ok(FfmpegBackend {
            ffmpeg: find("ffmpeg")?,
            ffprobe: find("ffprobe")?,
            version: OnceLock::new(),
        })
    }

    /// The version of `ffmpeg` (`ffmpeg -version`, read once); `None` for development builds
    /// without a release number, and if it could not be read: both are treated as new.
    pub fn version(&self) -> Option<FfmpegVersion> {
        *self.version.get_or_init(|| {
            let output =
                process::run("ffmpeg", &self.ffmpeg, ["-version"], &VERSION_LIMITS).ok()?;
            FfmpegVersion::parse(&String::from_utf8_lossy(&output.stdout))
        })
    }

    /// The version, or an error if it is older than [`FfmpegVersion::MINIMUM`]. Every operation
    /// checks first, so a too old FFmpeg says so instead of failing on an unknown option.
    fn checked_version(&self) -> Result<Option<FfmpegVersion>, MediaError> {
        match self.version() {
            Some(found) if found < FfmpegVersion::MINIMUM => {
                Err(MediaError::FfmpegTooOld { found })
            }
            version => Ok(version),
        }
    }

    /// Path of the `ffmpeg` executable.
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg
    }

    /// Path of the `ffprobe` executable.
    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe
    }
}

impl MediaBackend for FfmpegBackend {
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        // Opening a video probes it first: the place to tell that FFmpeg is too old.
        self.checked_version()?;
        // An absolute path can never be mistaken for a command-line option ("-…").
        let path = path::absolute(path).map_err(|_| MediaError::FileNotFound(path.into()))?;
        if !path.is_file() {
            return Err(MediaError::FileNotFound(path));
        }

        let options = [
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ];
        let args = options.iter().map(OsStr::new).chain([path.as_os_str()]);
        let output = process::run("ffprobe", &self.ffprobe, args, &PROBE_LIMITS)?;
        if !output.status.success() {
            return Err(tool_failed("ffprobe", &path, &output.stderr));
        }
        probe::parse_probe(&output.stdout, path)
    }

    fn decode_frame(
        &self,
        media: &MediaInfo,
        frame: i64,
        max_size: (u32, u32),
    ) -> Result<VideoFrame, MediaError> {
        self.checked_version()?;
        let frame = frame::clamp_frame(media, frame);
        let (width, height) = fit_within(media.video.display_size(), max_size);
        let expected = rgba_len(width, height).ok_or(MediaError::BadFrame {
            width,
            height,
            got: 0,
        })?;

        let seek = DecimalSecs(frame::seek_target(media, frame)).to_string();
        let scale = format!("scale={width}:{height}:flags=bilinear");
        // `-ss` before `-i` seeks in the input; ffmpeg then decodes from the previous keyframe
        // and drops frames before the target, so the first frame out is exactly `frame`.
        // `0:V:0` is the first real video stream (no cover pictures); rotation is applied
        // automatically before `scale`.
        let args = [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-ss"),
            OsStr::new(&seek),
            OsStr::new("-i"),
            media.path.as_os_str(),
            OsStr::new("-map"),
            OsStr::new("0:V:0"),
            OsStr::new("-frames:v"),
            OsStr::new("1"),
            OsStr::new("-vf"),
            OsStr::new(&scale),
            OsStr::new("-pix_fmt"),
            OsStr::new("rgba"),
            OsStr::new("-f"),
            OsStr::new("rawvideo"),
            OsStr::new("pipe:1"),
        ];
        let limits = Limits {
            timeout: FRAME_TIMEOUT,
            max_stdout: u64::try_from(expected)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
            max_stderr: 64 * 1024,
        };
        let output = process::run("ffmpeg", &self.ffmpeg, args, &limits)?;
        if !output.status.success() {
            return Err(tool_failed("ffmpeg", &media.path, &output.stderr));
        }
        if output.stdout.is_empty() {
            return Err(MediaError::NoFrame { frame });
        }
        let got = output.stdout.len();
        VideoFrame::new(width, height, output.stdout).ok_or(MediaError::BadFrame {
            width,
            height,
            got,
        })
    }

    fn video_stream(
        &self,
        media: &MediaInfo,
        from: i64,
        count: i64,
        max_size: (u32, u32),
    ) -> Result<Box<dyn VideoStream>, MediaError> {
        let [pass_option, pass_value] = FfmpegVersion::passthrough(self.checked_version()?);
        let from = frame::clamp_frame(media, from);
        let count = count.clamp(1, media.frame_count().saturating_sub(from).max(1));
        let (width, height) = fit_within(media.video.display_size(), max_size);
        let frame_len = rgba_len(width, height).ok_or(MediaError::BadFrame {
            width,
            height,
            got: 0,
        })?;

        let seek = DecimalSecs(frame::seek_target(media, from)).to_string();
        let count_arg = count.to_string();
        let scale = format!("scale={width}:{height}:flags=bilinear");
        // As in `decode_frame`; `passthrough` makes every decoded picture one output picture
        // (no duplicates or drops), so picture n of the stream is frame `from + n`.
        let args = [
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-nostdin"),
            OsStr::new("-ss"),
            OsStr::new(&seek),
            OsStr::new("-i"),
            media.path.as_os_str(),
            OsStr::new("-map"),
            OsStr::new("0:V:0"),
            OsStr::new("-frames:v"),
            OsStr::new(&count_arg),
            OsStr::new(pass_option),
            OsStr::new(pass_value),
            OsStr::new("-vf"),
            OsStr::new(&scale),
            OsStr::new("-pix_fmt"),
            OsStr::new("rgba"),
            OsStr::new("-f"),
            OsStr::new("rawvideo"),
            OsStr::new("pipe:1"),
        ];
        let process = Streaming::spawn("ffmpeg", &self.ffmpeg, args, stream::MAX_STDERR)?;
        Ok(Box::new(FfmpegVideoStream {
            process,
            path: media.path.clone(),
            size: (width, height),
            frame_len,
            ended: false,
        }))
    }

    fn audio_stream(
        &self,
        media: &MediaInfo,
        track: usize,
        (from, count): (i64, i64),
        delay: Tick,
        format: AudioFormat,
    ) -> Result<Box<dyn AudioStream>, MediaError> {
        self.checked_version()?;
        if track >= media.audio.len() {
            return Err(MediaError::NoAudioStream);
        }
        let map_arg = format!("0:a:{track}");
        let AudioFormat {
            sample_rate,
            channels,
        } = format;
        if !(1..=768_000).contains(&sample_rate) || !(1..=64).contains(&channels) {
            return Err(MediaError::BadAudioFormat {
                sample_rate,
                channels,
            });
        }
        let rate = media.video.frame_rate;
        let from = frame::clamp_frame(media, from);
        let start = rate.frame_to_tick(from);
        let end = rate.frame_to_tick(from.saturating_add(count.max(0)));
        let (seek, silence) = audio_source(start, delay);
        let duration_arg = DecimalSecs(end.saturating_sub(start)).to_string();
        let (channels_arg, rate_arg) = (channels.to_string(), sample_rate.to_string());
        // Audio seeks are sample-exact: ffmpeg decodes from before `-ss` and trims.
        let mut args: Vec<OsString> = vec![
            "-v".into(),
            "error".into(),
            "-nostdin".into(),
            "-ss".into(),
            DecimalSecs(seek).to_string().into(),
            "-i".into(),
            media.path.clone().into_os_string(),
            "-map".into(),
            map_arg.into(),
            "-vn".into(),
        ];
        if let Some(filter) = lead_silence_filter(silence) {
            args.extend(["-af".into(), filter.into()]);
        }
        args.extend(
            [
                "-t",
                &duration_arg,
                "-ac",
                &channels_arg,
                "-ar",
                &rate_arg,
                "-f",
                "f32le",
                "pipe:1",
            ]
            .map(OsString::from),
        );
        let process = Streaming::spawn("ffmpeg", &self.ffmpeg, &args, stream::MAX_STDERR)?;
        Ok(Box::new(FfmpegAudioStream {
            process,
            path: media.path.clone(),
            bytes: Vec::new(),
            ended: false,
            pending_error: None,
        }))
    }

    fn encoders(&self) -> Result<Encoders, MediaError> {
        self.checked_version()?;
        let limits = Limits {
            timeout: Duration::from_secs(15),
            max_stdout: 1024 * 1024,
            max_stderr: 64 * 1024,
        };
        let args = ["-hide_banner", "-v", "error", "-encoders"];
        let output = process::run("ffmpeg", &self.ffmpeg, args, &limits)?;
        if !output.status.success() {
            return Err(MediaError::ToolFailed {
                tool: "ffmpeg",
                message: process::last_line(&output.stderr),
            });
        }
        Ok(Encoders::parse(&String::from_utf8_lossy(&output.stdout)))
    }

    fn export(
        &self,
        media: &MediaInfo,
        job: &ExportJob,
        progress: &mut dyn FnMut(i64) -> bool,
    ) -> Result<ExportEnd, MediaError> {
        let args = export_args(media, job, self.checked_version()?)?;
        let output = path::absolute(&job.output).map_err(|source| MediaError::CreateOutput {
            path: job.output.clone(),
            source,
        })?;
        // Claim the name first: creating the file exclusively is what guarantees that no file is
        // ever overwritten, even one that appeared after the dialog checked the name.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
        {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(MediaError::OutputExists(output));
            }
            Err(source) => {
                return Err(MediaError::CreateOutput {
                    path: output,
                    source,
                });
            }
        }
        // `file:` keeps ffmpeg from reading a protocol out of the path; `-y` overwrites only the
        // empty file just created.
        let mut target = OsString::from("file:");
        target.push(output.as_os_str());
        let args = args
            .iter()
            .map(OsString::as_os_str)
            .chain([OsStr::new("-y"), target.as_os_str()]);

        let result = run_export(&self.ffmpeg, args, progress);
        match result {
            Ok(ExportEnd::Finished) => Ok(ExportEnd::Finished),
            other => {
                // Best effort: a partial file is worthless, but failing to remove it is no
                // reason to hide the real error.
                let _ = fs::remove_file(&output);
                other
            }
        }
    }
}

/// Runs the export ffmpeg and follows its progress output.
fn run_export<'a>(
    ffmpeg: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    progress: &mut dyn FnMut(i64) -> bool,
) -> Result<ExportEnd, MediaError> {
    let mut process = Streaming::spawn("ffmpeg", ffmpeg, args, stream::MAX_STDERR)?;
    let mut parser = ProgressParser::default();
    let mut buf = [0; 4096];
    loop {
        let n = process.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let frames = buf.get(..n).and_then(|bytes| parser.feed(bytes));
        if let Some(frames) = frames
            && !progress(frames)
        {
            // Dropping the process kills ffmpeg.
            return Ok(ExportEnd::Cancelled);
        }
    }
    let (status, stderr) = process.finish()?;
    if status.success() {
        Ok(ExportEnd::Finished)
    } else {
        Err(MediaError::ExportFailed {
            message: process::last_line(&stderr),
        })
    }
}

/// Valid audio bit rates in kbit/s.
const AUDIO_KBPS: std::ops::RangeInclusive<u32> = 8..=1024;

/// Every ffmpeg argument of an export except the output, for ffmpeg `version`.
fn export_args(
    media: &MediaInfo,
    job: &ExportJob,
    version: Option<FfmpegVersion>,
) -> Result<Vec<OsString>, MediaError> {
    if job.count < 1 {
        return Err(MediaError::BadExport("the range is empty"));
    }
    if let Some(audio) = &job.audio {
        if audio.track >= media.audio.len() {
            return Err(MediaError::NoAudioStream);
        }
        if !AUDIO_KBPS.contains(&audio.kbps) {
            return Err(MediaError::BadExport("audio bit rate out of range"));
        }
    }
    if job.hardware && media.video.rotation != 0 {
        return Err(MediaError::BadExport(
            "hardware acceleration cannot export rotated video; turn it off",
        ));
    }
    let rate = media.video.frame_rate;
    let from = frame::clamp_frame(media, job.from);
    let count = job
        .count
        .min(media.frame_count().saturating_sub(from))
        .max(1);
    let start = rate.frame_to_tick(from);
    let duration = rate
        .frame_to_tick(from.saturating_add(count))
        .saturating_sub(start);

    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    push(&["-v", "error", "-nostdin", "-nostats", "-progress", "pipe:1"]);
    push(&["-stats_period", "0.25"]);
    // Video from half a frame before the first frame (see `seek_target`), so that frame is the
    // first one out; audio sample-exact from the frame's start, as a second input.
    let video_seek = DecimalSecs(frame::seek_target(media, from)).to_string();
    if job.hardware {
        // Decoded pictures stay in GPU memory for NVENC (ffmpeg falls back to the CPU decoder
        // for codecs the GPU cannot decode; NVENC takes those pictures too).
        push(&["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    }
    push(&["-ss", &video_seek, "-i"]);
    args.push(media.path.clone().into_os_string());
    let mut lead_silence = None;
    if let Some(audio) = &job.audio {
        let (seek, silence) = audio_source(start, audio.delay);
        lead_silence = lead_silence_filter(silence);
        let audio_seek = DecimalSecs(seek).to_string();
        args.extend(["-ss".into(), audio_seek.into(), "-i".into()]);
        args.push(media.path.clone().into_os_string());
    }
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));

    push(&["-map", "0:V:0"]);
    let codec = job.video;
    // Exactly `count` pictures (the seek makes the first frame the first one out). A filter, not
    // `-frames:v`: before FFmpeg 5, reaching `-frames:v` ends every stream of the output, and
    // the audio track is left out of the file. The video starts at time 0 like the audio; 4:2:0
    // video (what every player decodes) needs even sizes, so an odd last row or column is
    // cropped.
    let video_filter =
        format!("trim=end_frame={count},setpts=PTS-STARTPTS,crop=trunc(iw/2)*2:trunc(ih/2)*2");
    push(&["-vf", &video_filter]);
    push(&FfmpegVersion::passthrough(version));
    if job.hardware {
        let cq = codec.hardware_cq(job.quality).to_string();
        push(&[
            "-c:v",
            codec.hardware_encoder(),
            "-preset",
            "p5",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            &cq,
            "-b:v",
            "0",
        ]);
    } else {
        let crf = codec.crf(job.quality).to_string();
        push(&["-c:v", codec.encoder(), "-crf", &crf, "-pix_fmt", "yuv420p"]);
        match codec {
            VideoCodec::H264 => push(&["-preset", "medium"]),
            VideoCodec::H265 => push(&["-preset", "medium", "-x265-params", "log-level=error"]),
            VideoCodec::Av1 => push(&["-preset", "8"]),
        }
    }
    if codec == VideoCodec::H265 {
        // The tag Apple players need for HEVC in MP4.
        push(&["-tag:v", "hvc1"]);
    }
    if let Some(audio) = &job.audio {
        let map = format!("1:a:{}", audio.track);
        let kbps = format!("{}k", audio.kbps);
        push(&["-map", &map, "-c:a", audio.codec.encoder(), "-b:a", &kbps]);
        if let Some(filter) = &lead_silence {
            push(&["-af", filter]);
        }
    }
    let duration = DecimalSecs(duration).to_string();
    push(&[
        "-t",
        &duration,
        "-map_chapters",
        "-1",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]);
    Ok(args)
}

/// Where the sound for a stream starting at `start` comes from with audio sync `delay`: the
/// source time to seek to and the silence to play before it (when the delay reaches back before
/// the start of the source).
fn audio_source(start: Tick, delay: Tick) -> (Tick, Tick) {
    let source = start.saturating_sub(delay);
    if source < Tick::ZERO {
        (Tick::ZERO, Tick::ZERO.saturating_sub(source))
    } else {
        (source, Tick::ZERO)
    }
}

/// The ffmpeg audio filter that plays `silence` before the sound (`adelay`, in milliseconds
/// with microsecond precision, on every channel); `None` without silence.
fn lead_silence_filter(silence: Tick) -> Option<String> {
    if silence <= Tick::ZERO {
        return None;
    }
    let micros = silence.0 / (TICKS_PER_SECOND / 1_000_000);
    Some(format!(
        "adelay=delays={}.{:03}:all=1",
        micros / 1000,
        micros % 1000
    ))
}

/// Error from the last stderr line. ffmpeg tools prefix errors with the input path, which the UI
/// already shows, so that prefix is removed.
pub(crate) fn tool_failed(tool: &'static str, path: &Path, stderr: &[u8]) -> MediaError {
    let line = process::last_line(stderr);
    let prefix = format!("{}: ", path.display());
    let message = line.strip_prefix(&prefix).unwrap_or(&line).to_owned();
    MediaError::ToolFailed { tool, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: i64) -> Tick {
        Tick::from_millis(n)
    }

    #[test]
    fn audio_sync_moves_the_source_or_adds_silence() {
        // Sound later: from earlier in the source.
        assert_eq!(audio_source(ms(5000), ms(150)), (ms(4850), Tick::ZERO));
        // Sound earlier: from later in the source.
        assert_eq!(audio_source(ms(5000), ms(-150)), (ms(5150), Tick::ZERO));
        // Before the start of the source: silence first.
        assert_eq!(audio_source(ms(100), ms(150)), (Tick::ZERO, ms(50)));
        assert_eq!(audio_source(Tick::ZERO, ms(150)), (Tick::ZERO, ms(150)));
        assert_eq!(audio_source(ms(100), Tick::ZERO), (ms(100), Tick::ZERO));
        // Hostile values saturate.
        assert_eq!(audio_source(Tick(i64::MAX), Tick(i64::MIN)).1, Tick::ZERO);
        assert!(audio_source(Tick(i64::MIN), Tick(i64::MAX)).1 > Tick::ZERO);
    }

    fn media() -> MediaInfo {
        MediaInfo {
            path: PathBuf::from("/videos/clip.mp4"),
            container: "mp4".to_owned(),
            start: Tick::ZERO,
            duration: Tick::from_secs(10),
            video: crate::VideoInfo {
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: microcut_time::FrameRate::new(25, 1).unwrap(),
                rotation: 0,
            },
            audio: Vec::new(),
        }
    }

    fn backend_with(version: Option<FfmpegVersion>) -> FfmpegBackend {
        FfmpegBackend {
            ffmpeg: PathBuf::from("no/such/ffmpeg"),
            ffprobe: PathBuf::from("no/such/ffprobe"),
            version: OnceLock::from(version),
        }
    }

    fn version(major: u32, minor: u32) -> Option<FfmpegVersion> {
        Some(FfmpegVersion { major, minor })
    }

    #[test]
    fn export_passes_pictures_through_on_every_version() {
        let job = ExportJob {
            output: PathBuf::from("/out/cut.mp4"),
            from: 0,
            count: 50,
            video: VideoCodec::H264,
            quality: crate::VideoQuality::High,
            hardware: false,
            audio: None,
        };
        let has = |args: &[OsString], pair: [&str; 2]| {
            args.windows(2).any(|w| {
                w.first().is_some_and(|a| a == pair[0]) && w.get(1).is_some_and(|b| b == pair[1])
            })
        };
        let old = export_args(&media(), &job, version(4, 4)).unwrap();
        assert!(has(&old, ["-vsync", "passthrough"]));
        assert!(!old.iter().any(|a| a == "-fps_mode"));
        for new in [version(5, 1), version(7, 1), None] {
            let args = export_args(&media(), &job, new).unwrap();
            assert!(has(&args, ["-fps_mode", "passthrough"]));
            assert!(!args.iter().any(|a| a == "-vsync"));
        }
    }

    #[test]
    fn export_counts_frames_with_a_filter() {
        // FFmpeg 4.4 drops the audio track when `-frames:v` ends the video; `trim` does not.
        let job = ExportJob {
            output: PathBuf::from("/out/cut.mp4"),
            from: 10,
            count: 50,
            video: VideoCodec::H264,
            quality: crate::VideoQuality::High,
            hardware: false,
            audio: None,
        };
        for version in [version(4, 4), version(7, 1)] {
            for hardware in [false, true] {
                let job = ExportJob {
                    hardware,
                    ..job.clone()
                };
                let args = export_args(&media(), &job, version).unwrap();
                assert!(!args.iter().any(|a| a == "-frames:v"), "{args:?}");
                let filter = args
                    .iter()
                    .skip_while(|a| *a != "-vf")
                    .nth(1)
                    .and_then(|f| f.to_str())
                    .unwrap();
                assert!(filter.starts_with("trim=end_frame=50,"), "{filter}");
            }
        }
    }

    #[test]
    fn too_old_ffmpeg_says_so_first() {
        let backend = backend_with(version(4, 3));
        let err = backend.probe(Path::new("/videos/clip.mp4")).unwrap_err();
        assert!(matches!(err, MediaError::FfmpegTooOld { .. }), "{err}");
        assert!(
            err.to_string()
                .starts_with("FFmpeg 4.3 is too old: MicroCut needs FFmpeg 4.4 or newer.")
        );
        assert!(matches!(
            backend.decode_frame(&media(), 0, (64, 64)),
            Err(MediaError::FfmpegTooOld { .. })
        ));
        assert!(matches!(
            backend.encoders(),
            Err(MediaError::FfmpegTooOld { .. })
        ));
        // Old enough, or a development build: on to the real work (here: the file is missing).
        for ok in [version(4, 4), None] {
            let err = backend_with(ok)
                .probe(Path::new("/no/such/clip.mp4"))
                .unwrap_err();
            assert!(matches!(err, MediaError::FileNotFound(_)), "{err}");
        }
    }

    #[test]
    fn lead_silence_is_an_adelay_filter() {
        assert_eq!(lead_silence_filter(Tick::ZERO), None);
        assert_eq!(lead_silence_filter(ms(-5)), None);
        assert_eq!(
            lead_silence_filter(ms(150)).as_deref(),
            Some("adelay=delays=150.000:all=1")
        );
        // A 60 fps frame start: 16.666 ms.
        let frame = Tick(TICKS_PER_SECOND / 60);
        assert_eq!(
            lead_silence_filter(frame).as_deref(),
            Some("adelay=delays=16.666:all=1")
        );
    }
}
