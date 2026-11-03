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

## M4 — Playback

- [ ] M4.1 Play/pause from the cursor: stream rawvideo frames from ffmpeg, paced by the clock; cursor moves along
- [ ] M4.2 Audio playback (`cpal`), video synced to the audio clock

## M5 — Export

- [ ] M5.1 Set start/end (buttons + `I`/`O` keys)
- [ ] M5.2 Export with the fixed encoder (H.264 `libx264` CRF 18 `preset medium`, AAC 192 kb/s, `.mp4`) next to the
  original as `<stem>_hh-mm-ss-to-hh-mm-ss.mp4`; never overwrite (append ` (2)`, …)
- [ ] M5.3 Progress bar (`-progress pipe:1`), cancel, error display

## Later

- Remember window size and last folder
- Control channel for agent-driven UI tests and screenshots
