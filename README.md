# MicroCut

A minimal video trimmer in Rust: open a video, pick a start and an end on the timeline, click **Export**. The cut is
written next to the original, re-encoded with a fixed encoder, named `<original>_hh-mm-ss-to-hh-mm-ss.mp4`.

Status: **M4 — playback.** Open a file (dialog, drag & drop or `microcut <video>`), find your way on the timeline,
select the range to keep and play it with sound; the preview always shows the exact frame at the cursor. Export comes
next; see [`ROADMAP.md`](ROADMAP.md).

| Input                                  | Action                                         |
| -------------------------------------- | ---------------------------------------------- |
| Space / ▶ button / click on the picture | Play from the cursor / pause                  |
| Mouse wheel / `+` `−`                  | Zoom the timeline (at the pointer / cursor)    |
| Drag the timeline or its scrollbar, Shift+wheel | Scroll the timeline                   |
| Click on the timeline                  | Move the cursor                                |
| Drag a range handle (frame scale)      | Move the start or end of the range (Shift: snap to the time scale) |
| Click / drag an edge arrow (frame scale) | Bring a range end that is out of view to the edge of the view / take hold of it |
| Double-click the time scale or scrollbar | Show the whole clip                          |
| `I` / `O`, "Set start" / "Set end"     | Start / end the range at the cursor            |
| ←/→ or A/D                             | One frame (keys pause playback)                |
| ↑/↓ or W/S (or Shift+←/→), Page Up/Down | One second, ten seconds                       |
| Home/End                               | First/last frame                               |
| Speaker / wedge / wheel (status line)  | Mute, set volume                               |
| `[0] [1] …` left of the speaker        | Choose the audio track (videos with several)   |
| Ctrl+O                                 | Open a video                                   |

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
