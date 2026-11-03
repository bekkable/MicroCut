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
│ [Open…] path                     media facts │
├──────────────────────────────────────────────┤
│                                              │
│            preview (frame at cursor)         │
│                                              │
├──────────────────────────────────────────────┤
│ ▶  00:01:05.12  range …   [Start] [End] [Export] │
├──────────────────────────────────────────────┤
│ frame scale ····|····[=====range=====]····   │
│ ▮▮▮▮▮▮▮▮▮▮▮ video strip (thumbnails) ▮▮▮▮▮▮▮ │  ← mouse wheel zooms
│ time scale  0:00     0:30     1:00     1:30  │
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
- **Export** writes `<stem>_hh-mm-ss-to-hh-mm-ss.mp4` next to the original (see `ROADMAP.md` M5.2).

## 3. Threads

The UI thread never waits on ffmpeg (`apps/microcut/src/loader.rs` is the pattern: worker thread, `catch_unwind`,
channel, `request_repaint`). Every ffmpeg/ffprobe run has a time limit and capped output
(`crates/media/src/process.rs`) and no console window on Windows. Decode, playback and export run on worker threads
(under `catch_unwind`) and send results over channels; the UI requests a repaint when a result arrives. While scrubbing
only the newest frame request is served: the preview worker (`apps/microcut/src/preview.rs`) reads a one-slot mailbox
that each new request overwrites. Thumbnails (`apps/microcut/src/thumbs.rs`) use two workers and a queue that the UI
replaces with the currently visible, not yet decoded frames on every repaint, so zooming and scrolling never leave a
backlog of invisible work.

## 4. Frame-exact seeking

`MediaBackend::decode_frame` takes a frame index, not a time. The ffmpeg backend seeks with `-ss` before `-i` to half
a frame *before* the frame's start (`crates/media/src/frame.rs`, `seek_target`): ffmpeg decodes from the previous
keyframe and drops frames before the target, and container timestamps may be rounded (Matroska: 1 ms), so aiming at
the exact start could skip the frame. `crates/media/tests/decode.rs` proves it with fixtures whose frames encode their
own number in their brightness.

The frame count comes from the video stream's duration when the container has one (the container duration also covers
longer audio), else from the container duration.
