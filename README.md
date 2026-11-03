# MicroCut

A minimal video trimmer in Rust: open a video, pick a start and an end on the timeline, click **Export**. The cut is
written next to the original, re-encoded with a fixed encoder, named `<original>_hh-mm-ss-to-hh-mm-ss.mp4`.

Status: **M0 — skeleton.** The app opens a window that says "Hello World!". See [`ROADMAP.md`](ROADMAP.md).

## Build and run

```sh
cargo run            # start the app
cargo xtask ci       # fmt, clippy, tests
```

Needs Rust ≥ 1.95 and, from M1 on, `ffmpeg`/`ffprobe` in `PATH`.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. Use it for any
purpose, commercial or not.

MicroCut runs FFmpeg as a separate program; FFmpeg is not part of MicroCut and keeps its own license (LGPL/GPL, see
[`NOTICE`](NOTICE)).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in MicroCut by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
