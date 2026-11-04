# MicroCut

A minimal video trimmer in Rust: open a video, pick a start and an end on the timeline, click **Export**. The cut is
re-encoded and written next to the original as `<original>_hh-mm-ss-to-hh-mm-ss.mp4`; existing files are never
overwritten.

Status: **M5 — export.** Open a file (dialog, drag & drop or `microcut <video>`), find your way on the timeline,
select the range to keep, play it with sound and export it frame-exactly as MP4 (H.264, H.265 or AV1; AAC or Opus;
one audio track of your choice); the preview always shows the exact frame at the cursor. See
[`ROADMAP.md`](ROADMAP.md).

| Input                                  | Action                                         |
| -------------------------------------- | ---------------------------------------------- |
| Space / ▶ button / click on the picture | Play from the cursor / pause                  |
| Mouse wheel / `+` `−`                  | Zoom the timeline (at the pointer / cursor)    |
| Drag the timeline or its scrollbar, Shift+wheel | Scroll the timeline                   |
| Click on the timeline                  | Move the cursor                                |
| Drag a range handle (frame scale)      | Move the start or end of the range (Shift: snap to the time scale) |
| Click a range handle                   | Cursor to that end of the range                |
| Click / drag an edge arrow (frame scale) | Bring a range end that is out of view to the edge of the view / take hold of it |
| Double-click the time scale or scrollbar | Show the whole clip                          |
| `I` / `O`, "Set start" / "Set end"     | Start / end the range at the cursor            |
| ←/→ or A/D                             | One frame (keys pause playback)                |
| ↑/↓ or W/S (or Shift+←/→), Page Up/Down | One second, ten seconds                       |
| Home/End                               | Start/end of the range                         |
| Ctrl+Home/End                          | First/last frame of the clip                   |
| Speaker / wedge / wheel (status line)  | Mute, set volume                               |
| `[0] [1] …` left of the speaker        | Choose the audio track (videos with several)   |
| Ctrl+O                                 | Open a video                                   |
| Export…, E, Ctrl+E                     | Export the range (codecs, quality, audio track, file name; optionally on an NVIDIA graphics card) |

## Build and run

```sh
cargo run            # start the app
cargo xtask ci       # fmt, clippy, tests
```

Needs Rust ≥ 1.95 to build and [FFmpeg](https://ffmpeg.org) (`ffmpeg` and `ffprobe`) to run: in `PATH`, next to the
MicroCut executable, or in the folder named by `MICROCUT_FFMPEG_DIR`.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Use it for any
purpose, commercial or not.

MicroCut runs FFmpeg as a separate program; FFmpeg is not part of MicroCut and keeps its own license (LGPL/GPL, see
[`NOTICE`](NOTICE)).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in MicroCut by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
