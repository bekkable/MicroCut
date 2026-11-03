# ADR 0001: ffmpeg as media backend, run as an external process

Status: accepted (M0)

## Context

MicroCut must decode frames for preview and playback and encode the exported cut. Rather than writing its own
codecs, MicroCut uses ffmpeg. Two ways to use it:

1. **External process** — spawn `ffmpeg`/`ffprobe`, read JSON (probe), raw RGBA frames (preview, playback) and
   progress lines (export) from pipes.
2. **Library** — link libavformat/libavcodec via `ffmpeg-next` / `ffmpeg-sys-next`.

## Decision

Start with the external process (1), behind a `MediaBackend` trait in `crates/media`, so a library backend (2) can be
added later without touching the UI or engine.

## Consequences

- No native build dependencies: no ffmpeg dev headers, no clang/bindgen, no `unsafe` FFI in the workspace. The build
  works with a plain `cargo build`.
- ffmpeg stays a separate program: no LGPL/GPL linking obligations for MicroCut itself. Users need ffmpeg installed
  (or shipped next to the binary, with its license).
- Seeking per preview frame costs a process start (~tens of ms). Fine for scrubbing with "latest request wins";
  playback uses one long-running process per play.
- Revisit (2) if scrubbing latency is too high. Then: `ffmpeg-next` behind a cargo feature, FFI contained in one crate,
  and a license check of the linked ffmpeg build (a GPL build makes the shipped binary GPL).
