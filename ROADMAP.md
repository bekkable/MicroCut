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

- [ ] M5.1 Set start/end (buttons + `I`/`O` keys)
- [ ] M5.2 Export with the fixed encoder (H.264 `libx264` CRF 18 `preset medium`, AAC 192 kb/s, `.mp4`) next to the
  original as `<stem>_hh-mm-ss-to-hh-mm-ss.mp4`; never overwrite (append ` (2)`, …). Open question for the user:
  with several audio tracks, export the selected one, all of them, or a mix?
- [ ] M5.3 Progress bar (`-progress pipe:1`), cancel, error display

## Later

- Remember window size and last folder
- Control channel for agent-driven UI tests and screenshots
