# Roadmap

Each task has an id (`M<milestone>.<n>`); one task per commit.

## M0 — Skeleton

- [x] M0.1 Workspace, lints, `cargo xtask ci`, agent docs
- [x] M0.2 `microcut-time`: `Tick`, `FrameRate`, `range_label`
- [x] M0.3 App window showing "Hello World!"
- [x] M0.4 License (MIT OR Apache-2.0), NOTICE on FFmpeg, git repository

## M1 — Open a video

- [x] M1.1 `crates/media`: `MediaBackend` trait + ffmpeg process backend; locate `ffmpeg`/`ffprobe`, clear error if
  missing
- [x] M1.2 Probe via `ffprobe -print_format json`: duration, frame rate, size, rotation, audio (hostile-input parsing)
- [x] M1.3 Open via file dialog (`rfd`), drag & drop and command-line argument

## M2 — Preview

- [x] M2.1 Decode the frame at the cursor (`ffmpeg -ss … -frames:v 1 -f rawvideo -pix_fmt rgba pipe:1`), scaled to
  the preview size, on a background thread — frame-exact (tested in MP4 and Matroska), ~120–320 ms per 720p frame
- [x] M2.2 Show it in the preview area (top), aspect-correct, rotation applied by ffmpeg; latest request wins while
  scrubbing. Until M3 the cursor is moved by a temporary frame slider and ←/→ (frame), Shift+←/→ (second),
  Home/End

## M3 — Timeline (bottom of the window)

- [x] M3.1 Video strip with thumbnails (two decoder threads, only visible tiles queued, LRU cache), horizontal
  scrolling (Shift+wheel, horizontal wheel/touchpad, dragging past an edge, cursor keys scroll along)
- [x] M3.2 Mouse-wheel zoom anchored at the pointer, from whole clip to single frames (120 px per frame); `+`/`−`
  zoom around the cursor, double-click on the time scale shows the whole clip
- [x] M3.3 Time scale below the strip (adaptive tick spacing, `hh:mm:ss` labels, `hh:mm:ss.mmm` below one second)
- [x] M3.4 Frame scale above the strip with the selected range (start/end handles, draggable; the preview shows the
  frame at the dragged handle); the whole clip is selected after opening
- [x] M3.5 Cursor: click/drag to select a frame; preview follows. Replaces the temporary slider from M2.2 (the arrow
  key bindings stay)
- [x] M3.6 Feedback: dragging the strip or a scale moves the view (a click sets the cursor); arrows at the edges of
  the frame scale for range ends out of view — a click brings that end to the cursor; scrollbar below the time
  scale (thumb = visible part, draggable, click on the track centres the view there, wheel zooms)
- [x] M3.7 Feedback: Shift while dragging a range handle snaps to the time scale's marks (seconds etc., by zoom);
  edge arrows: click moves the nearest hidden range end to the edge of the view, drag takes hold of it;
  double-click on the scrollbar zooms out to the whole clip

## M4 — Playback

- [x] M4.1 Play/pause from the cursor (button, Space): stream rawvideo frames from ffmpeg, paced by the clock;
  cursor moves along and stays where playback stopped. Started inside the range, playback stops at its end, else
  at the end of the clip. Moving the cursor while playing continues from there (after a drag: on release)
- [x] M4.2 Audio playback (`cpal`, default output device in its own format), video synced to the audio clock
  (samples actually played); system clock without sound or after the sound ended
- [x] M4.3 Feedback: navigation keys pause playback; ↑/↓ one second, Page Up/Down ten seconds; a click on the
  preview plays/pauses; small volume control (speaker = mute, wedge, wheel) on the right of the status line
- [x] M4.4 Feedback: the volume percentage is drawn inside the wedge (no layout jumps); WASD work like the arrow
  keys
- [x] M4.5 Several audio tracks (e.g. OBS "Mic" / "Music"): numbered buttons `[0] [1] …` left of the speaker switch
  the track played (also while playing); tooltip with title, language and format

## M5 — Export

- [x] M5.1 Set start/end at the cursor (buttons "Set start"/"Set end" in the status line, `I`/`O` keys); a start
  after the end (an end before the start) drops the other end
- [x] M5.2 Backend export: frame-exact re-encode of a frame range into MP4 with a chosen video codec and quality
  (H.264, H.265, AV1 — only encoders the installed FFmpeg has) and audio codec, bit rate and track (or none);
  progress via `-progress pipe:1`, cancellable; the output file is created exclusively (never overwrites) and
  removed again on failure or cancel
- [x] M5.3 Export dialog (Export… button, Ctrl+E): codecs, quality, audio track (a track titled "Music" is
  preselected, else the first; or none); frame count, duration and estimated size; file name
  `<stem>_hh-mm-ss-to-hh-mm-ss.mp4` next to the original, editable, ` (2)` appended if taken; buttons Cancel /
  Encode (Enter); codec choices are remembered for the next export
- [x] M5.4 Progress in the dialog (bar, frames, time left), cancel, errors; afterwards "Exported <name>" with "Show
  in folder" in the toolbar
- [x] M5.5 Feedback: a click on a range handle moves the cursor to that end; Home/End jump to the start/end of the
  range (Ctrl+Home/End: first/last frame of the clip); `E` opens the export dialog; export option "Hardware
  acceleration (NVIDIA)": CUDA decoding (`-hwaccel cuda -hwaccel_output_format cuda`) and NVENC encoding
  (`h264_nvenc`/`hevc_nvenc`, constant quality `-cq`), only for codecs with an NVENC encoder and unrotated video
- [x] M5.6 NVENC quality levels calibrated with VMAF against x264/x265 (H.264 CQ 27/31/36, H.265 CQ 28/33/38: same
  quality, about the same size); size estimate recalibrated on measured exports
- [x] M5.7 Audio sync: `Sync ◀ [+0 ms] ▶` left of the audio tracks in the status line and in the export dialog —
  milliseconds typed in (+ = sound later, ±10 s) or ±50 ms per arrow; shifts the sound in playback and export
  (silence where it reaches back before the start of the video); kept per opened video
- [x] M5.8 Feedback: encoding speed (fps) in the progress line; after the export the dialog shows the result instead
  of the toolbar note — file name and folder, size and bit rate, frames, length, picture size, codec settings, audio
  track, encoding time, fps and speed against real time; "Show in folder" and "Close" (Enter). Fix: a duration that
  ffprobe rounds to microseconds no longer loses the last frame (`MediaInfo::frame_count`)
- [x] M5.9 The export codecs (video codec, quality, hardware acceleration, audio codec and bit rate) are kept between
  runs (eframe storage: `%APPDATA%\microcut\data\app.ron`, `~/Library/Application Support/microcut`,
  `~/.local/share/microcut`); file name, audio track and sync are not, they belong to one video. An unreadable or
  edited file falls back to the defaults

## M6 — Release

FFmpeg is not shipped: users install it themselves (see `NOTICE`).

- [x] M6.1 `cargo xtask dist`: release archive for the platform it runs on — Windows x86_64 `.zip` (exe), macOS
  `.zip` with `MicroCut.app` (universal binary: Apple Silicon + Intel, ad-hoc signed), Linux x86_64 `.tar.gz`; each
  with README, licenses and NOTICE, in `dist/`. FFmpeg is also searched in the usual Homebrew/MacPorts folders on
  macOS (apps started from the Finder do not get the shell's `PATH`)
- [x] M6.2 GitHub Actions: `ci.yml` (quality gates on Windows, macOS and Linux for pushes and pull requests),
  `release.yml` (a tag `v*` builds the three archives, adds `SHA256SUMS.txt` and opens a draft release)
- [x] M6.3 App icon: original artwork, in the window, the Windows exe and the macOS bundle
- [x] M6.4 Older FFmpeg: the backend reads the version (`ffmpeg -version`) once; before 5.1 it passes
  `-vsync passthrough` instead of `-fps_mode passthrough`, and the export counts frames with `trim` instead of
  `-frames:v` (which drops the audio track before FFmpeg 5), so FFmpeg 4.4 (Ubuntu 22.04) works; before 4.4 a clear
  error says to install a newer FFmpeg. CI also runs the gates on Ubuntu 22.04 with its FFmpeg 4.4
- [ ] M6.5 Build provenance: the release workflow attests the archives (`actions/attest`, Sigstore, no key of our
  own); `gh attestation verify <archive> -R Najoko/MicroCut` checks a download. Ticked after the first release
- [ ] M6.6 macOS back in CI and releases: paused because GitHub's macOS runners never picked up the jobs (three runs
  on 2026-11-04, "not acquired … capacity constraints"). Add `macos-latest` to the matrices in `ci.yml` and
  `release.yml` again; the first run also tests the untried macOS part of `cargo xtask dist` (`lipo`, `codesign`,
  icns, `ditto`)

## M7 — Polish

- [ ] M7.1 "About MicroCut" dialog
- [x] M7.2 Second jumps are absolute: ↑/↓, W/S, Shift+←/→ go to the next/previous whole second since the start of
  the clip (4:45.234 → 4:46.000, not 4:46.234), Page Up/Down to the next/previous whole ten seconds; a multiple that
  falls between frames goes to the nearer frame boundary, and every jump moves at least one frame
- [ ] M7.3 Drag the cursor: near the cursor line the pointer becomes a horizontal-move cursor, and a drag moves the
  cursor instead of the view
- [ ] M7.4 Frame buffer while paused: the frames around the cursor are decoded in the background, so stepping
  frame by frame shows them at once

## Later

- Remember window size and last folder
- Control channel for agent-driven UI tests and screenshots
