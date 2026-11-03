# MicroCut — instructions for agents

MicroCut is a minimal, open-source Rust tool that trims a video to a start and an end point and exports the cut. It is
deliberately small: one tool for one job.
Desktop app on Windows, macOS and Linux (egui/eframe).

## Start every session here

1. Read [`ROADMAP.md`](ROADMAP.md): current milestone and the next unchecked task.
2. Read [`docs/architecture.md`](docs/architecture.md) (layers, crates, UI layout) and the ADRs in
   [`docs/adr/`](docs/adr/) that touch your task.
3. Work the task to a green state, tick it in `ROADMAP.md`, commit. Don't stop to ask unless the decision is the
   user's (product behaviour, licensing, new external dependencies with native code).

## Non-negotiables

- **Never crash.** A crash loses the user's cut. No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/
  `unimplemented!` or `unsafe` outside tests (clippy denies them workspace-wide): return `Result<T, E>` and propagate
  with `?` (`ok_or`, `let … else`, `if let`). Treat every number from media, ffprobe output or CLI params as hostile:
  `get()` instead of `[i]`, checked/saturating math, no division by possibly-zero values, capped allocations.
  `lock().unwrap_or_else(PoisonError::into_inner)`. Background jobs (decoding, export) run under `catch_unwind` and
  report errors to the UI. `apps/microcut/src/crash.rs` is the last-resort guard, not a licence. Every crash fix gets a
  regression test that panicked before.
- **Exact time:** all edit time is `microcut_time::Tick` (254 016 000 000/s); frame positions go through
  `FrameRate`. Never use `f64` seconds for cut points. `f64` is fine only for drawing (pixels per tick).
- **ffmpeg is the media backend** ([`docs/adr/0001-ffmpeg-backend.md`](docs/adr/0001-ffmpeg-backend.md)): probing,
  frame decoding, playback and export go through `ffmpeg`/`ffprobe`, behind the backend trait in `crates/media`. No
  other crate spawns ffmpeg or parses its output.
- **Layering** ([`docs/architecture.md`](docs/architecture.md) §1): nothing below L5 depends on egui/eframe/winit/rfd/
  cpal. L0 crates have no dependencies on other MicroCut crates.
- **License:** MicroCut is `MIT OR Apache-2.0` and must stay usable without restrictions. Dependencies must be
  permissive (MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0, Unicode, CC0; OFL-1.1/Ubuntu-font for bundled fonts); an `OR`
  expression is fine if one choice is permissive. Never link or vendor copyleft-only code (GPL, LGPL, AGPL, MPL): FFmpeg
  is only ever run as a separate process (see [`NOTICE`](NOTICE)). Ask the user before adding a dependency with a
  license outside that list.
- **Assets** (icons, sounds, test media) must be openly licensed or original; never commit media files — integration
  tests generate fixtures with ffmpeg into `$CARGO_TARGET_TMPDIR/fixtures/` (= `target/tmp/fixtures/`) and skip with a
  note when FFmpeg is missing.
- **UI text is English.** User-facing errors come from the `Display` of the error types (`MediaError`), so write them
  as sentences a user understands, with what to do next.
- **Quality gates** before every commit: `cargo xtask ci` (= `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`).
- **New workspace crates** live in `crates/<name>` as `microcut-<name>`, add themselves to `[workspace.dependencies]`
  and include `[lints] workspace = true`. Crate roots start with `#![forbid(unsafe_code)]`.
- **Commits:** one roadmap task id per commit (`M2.1: decode frame at cursor`). Only green states. End messages with
  the attribution line required by the environment.

## Running

- `cargo run` starts the app (`apps/microcut` is the default member); `cargo run -- <video>` opens a video directly.
- `cargo run -p microcut-media --example probe -- <video>…` prints what the backend reads, without the UI.
- `cargo xtask ci` runs all quality gates.
- Requires `ffmpeg` and `ffprobe` at runtime, searched in `MICROCUT_FFMPEG_DIR`, next to the executable, then `PATH`.
