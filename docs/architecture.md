# Architecture

## 1. Layers

A crate may depend only on crates of a lower layer.

| Layer | Crate               | Purpose                                                                  | Status |
| ----- | ------------------- | ------------------------------------------------------------------------ | ------ |
| L0    | `microcut-time`     | `Tick`, `FrameRate`, timecode / file-name formatting                     | M0     |
| L1    | `microcut-media`    | `MediaBackend` trait; ffmpeg backend: probe (M1), frame decode, play, export | M1     |
| L2    | `microcut-engine`   | Session state (file, cursor, range, zoom), scale marks, thumbnail tiles  | M3     |
| L5    | `apps/microcut`     | eframe app: window, panels, input, texture upload                        | M0     |

Nothing below L5 depends on egui/eframe/winit/rfd/cpal, so the engine and media crates stay testable headless.

## 2. Window layout

```
┌──────────────────────────────────────────────┐
│ [Open…] [Export…] path           media facts │
├──────────────────────────────────────────────┤
│                                              │
│            preview (frame at cursor)         │
│                                              │
├──────────────────────────────────────────────┤
│ ▶ 00:01:05 [Set start][Set end] … Sync ◀ +0 ms ▶ [0][1] 🔈◢ │
├──────────────────────────────────────────────┤
│◀frame scale ····|····[=====range=====]····  ▶│  ← arrows: range end out of view
│ ▮▮▮▮▮▮▮▮▮▮▮ video strip (thumbnails) ▮▮▮▮▮▮▮ │  ← wheel zooms, drag scrolls
│ time scale  0:00     0:30     1:00     1:30  │
│ ░░░░░░[▓▓▓]░░░░░░░░ scrollbar ░░░░░░░░░░░░░░ │  ← visible part of the clip
└──────────────────────────────────────────────┘
```

- **Preview** always shows the frame at the cursor. Play starts at the cursor.
- **Timeline** at the bottom (`apps/microcut/src/timeline.rs`): frame scale with range selection on top, video
  strip in the middle, time scale below. The mouse wheel zooms around the pointer position. The geometry — zoom and
  scroll (`View`), scale marks, thumbnail tiles — lives in `microcut-engine` and is unit-tested; the app only draws
  it and turns input into `Session` calls.
- **Thumbnail tiles** span 1, 2, 3, 4, 6, 8, 12, … frames and show their first frame, so the frames shown do not
  change while scrolling and recur at neighbouring zoom levels (cached thumbnails stay useful). A tile is at most
  1.5 thumbnails wide; thumbnails are decoded 1.5× larger and cropped to fill it.
- **Export** (`apps/microcut/src/export.rs`) is a modal dialog: codecs and quality (only encoders the installed
  FFmpeg has), audio track, an estimate of the size, the file name (default `<stem>_hh-mm-ss-to-hh-mm-ss.mp4` next to
  the original, ` (2)` if taken). The codec choices are kept between runs (`MicroCutApp::save`, eframe's storage,
  key `export`, written as `video=h264;quality=high;…` by `Choices::to_setting`; nothing else is saved, not even the
  window); file name, audio track and sync start fresh for each video. The name rules, the preselected track and the
  estimate live in `crates/engine/src/export.rs`. While it runs, the dialog shows progress (frames, fps, time left) and
  can cancel; closing the app cancels too. Afterwards the same dialog shows what was written
  (`apps/microcut/src/report.rs`: the file probed again, size, bit rate, settings, encoding time and speed) with "Show
  in folder" and "Close".

## 3. Threads

The UI thread never waits on ffmpeg (`apps/microcut/src/loader.rs` is the pattern: worker thread, `catch_unwind`,
channel, `request_repaint`). Every ffmpeg/ffprobe run has a time limit and capped output
(`crates/media/src/process.rs`) and no console window on Windows. Decode, playback and export run on worker threads
(under `catch_unwind`) and send results over channels; the UI requests a repaint when a result arrives. While scrubbing
only the newest frame request is served: the preview worker (`apps/microcut/src/preview.rs`) reads a one-slot mailbox
that each new request overwrites. Thumbnails (`apps/microcut/src/thumbs.rs`) use two workers and a queue that the UI
replaces with the currently visible, not yet decoded frames on every repaint, so zooming and scrolling never leave a
backlog of invisible work.

Playback (`apps/microcut/src/playback.rs`) reads two ffmpeg streams (`MediaBackend::video_stream` /
`audio_stream`, `crates/media/src/stream.rs`) on two worker threads: pictures go through a channel of six (the
decoder blocks when it is ahead), samples into a one-second queue that the `cpal` device callback empties
(`apps/microcut/src/audio.rs`; the callback never blocks — it plays silence if the queue is locked or empty). The
samples the device has played are the clock; the UI shows the newest picture the clock has reached and drops late
ones. The clock starts with the first picture, so ffmpeg's start-up time never makes the video jump. Without sound
(no audio stream, no device, decoder error) or once the sound ended, the system clock continues. Dropping the
playback kills both ffmpeg processes.

## 4. Frame-exact seeking

`MediaBackend::decode_frame` takes a frame index, not a time. The ffmpeg backend seeks with `-ss` before `-i` to half
a frame *before* the frame's start (`crates/media/src/frame.rs`, `seek_target`): ffmpeg decodes from the previous
keyframe and drops frames before the target, and container timestamps may be rounded (Matroska: 1 ms), so aiming at
the exact start could skip the frame. `crates/media/tests/decode.rs` proves it with fixtures whose frames encode their
own number in their brightness.

Playback streams seek the same way and pass every decoded picture through unchanged (`-fps_mode passthrough`; `-vsync`
before FFmpeg 5.1), so picture n of a stream is frame `from + n` — exact for constant-frame-rate video, which
`crates/media/tests/stream.rs` checks in MP4 and Matroska. Variable-frame-rate video plays with pictures shown at the
nominal rate.

Export (`MediaBackend::export`, `crates/media/src/ffmpeg.rs`) re-encodes, so it can cut at any frame: the video input
seeks like playback and is limited to the frame count (`trim=end_frame=…` in the filter, not `-frames:v`: before
FFmpeg 5, reaching `-frames:v` ends every stream of the output and the file has no audio track); the audio track is
opened as a second input of the same file, seeked sample-exactly to the first frame's start, and both are cut to the
range's duration (`-t`). The output file is created exclusively before ffmpeg runs (`-y` then only overwrites that empty
file), so an existing file is never replaced, and it is removed again on failure or cancel. Progress comes from
`-progress pipe:1`. With hardware acceleration the video input gets `-hwaccel cuda -hwaccel_output_format cuda` and the
encoder is NVENC (`-rc vbr -cq`): pictures stay on the GPU, so a software encoder could not take them; `setpts` and
`crop` work on GPU frames, the automatic rotation does not (rotated video is refused). The NVENC quality values
(`VideoCodec::hardware_cq`) are calibrated with VMAF: per quality level, the CQ at which NVENC (preset p5) matches the
VMAF of x264/x265 at the level's CRF, on four 10 s scenes of 720p60 recordings (menu and gameplay). VMAF needs a
reference with exactly the same frames: a lossless (`-qp 0`) encode of the range, compared frame by index
(`settb=1/60,setpts=N` on both inputs) — comparing by timestamps mis-pairs frames of Matroska sources (1 ms timestamps)
and drags even identical videos down to ~89. `crates/media/tests/export.rs` checks frame numbers, the audio track, every
available codec, cancel and failure.

Audio sync (`Session::audio_delay`, milliseconds in the UI) shifts the sound in playback and export alike
(`MediaBackend::audio_stream` and `AudioExport::delay`): a stream starting at time `t` takes its sound from `t − delay`
in the source; where that reaches back before the start of the source, `adelay` plays silence first. The picture
stays frame-exact and the sound covers the same span (it ends early only where the source runs out);
`tests/stream.rs` and `tests/export.rs` measure where a tone that starts at 1 s ends up.

The frame count comes from the video stream's duration when the container has one (the container duration also covers
longer audio), else from the container duration.

FFmpeg versions (`crates/media/src/version.rs`): the backend reads `ffmpeg -version` once, on first use (on a worker
thread, never while locating the programs). 4.4 is the minimum (`-stats_period`, NVENC presets `p1`–`p7`); anything
older fails every operation with `MediaError::FfmpegTooOld`, which says to install a newer FFmpeg. Before 5.1 the
pass-through option is `-vsync passthrough` (`-fps_mode` did not exist yet, and is the only option newer than 4.4).
Export also avoids `-frames:v` (see §4), checked with FFmpeg 4.4 in an `ubuntu:22.04` container: frame count, audio
length and tone onset match FFmpeg 6. Development builds without a release number count as new. CI runs the integration
tests with FFmpeg 4.4 (Ubuntu 22.04) and a current one.

## 5. Release

`cargo xtask dist` (`xtask/src/dist.rs`) builds the release binary and packs it with README, licenses and NOTICE into
`dist/microcut-<version>-<platform>.<zip|tar.gz>`, using only the system's tools: Windows' own `tar.exe` writes the
zip, macOS gets `MicroCut.app` (both architectures merged with `lipo`, `Info.plist`, ad-hoc `codesign`, zipped by
`ditto`), Linux a `.tar.gz` (keeps the executable bit). FFmpeg is not included (`NOTICE`); an app started from the
macOS Finder has a minimal `PATH`, so the backend also looks in the Homebrew and MacPorts folders
(`crates/media/src/locate.rs`).

The icon is original artwork drawn by `apps/microcut/assets/make_icon.py` (Pillow), which writes `icon.png` (1024 px)
and `icon.ico`; both are committed. The window gets `icon.png` at run time, `apps/microcut/build.rs` puts `icon.ico`
and the version information into the Windows exe (`winresource`, the SDK's `rc.exe`; without it the build only warns),
and `cargo xtask dist` turns `icon.png` into the bundle's `MicroCut.icns` (`sips`, `iconutil`).

GitHub Actions: `.github/workflows/ci.yml` runs `cargo xtask ci` on Windows, macOS, Ubuntu 22.04 (FFmpeg 4.4) and
Ubuntu 24.04 with FFmpeg installed (pushes to `main`, pull requests). `.github/workflows/release.yml` runs on a tag
`v<version>`: it checks the tag against the version in `Cargo.toml`, runs the CI workflow, builds the three archives
(Linux on Ubuntu 22.04 for an older glibc) and creates a draft release with them and `SHA256SUMS.txt`, to be published
by hand.
