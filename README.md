# ClipCraft

**A local-first Rust desktop app for instant game clips, playback, a searchable clip library, and
quick trims.** The current build captures and manages clips offline; it has no account, hosted
backend, backup, upload, sync, or share-link service.

> **Status: active prototype.** The screen-capture/replay/save pipeline, playback, local library,
> and basic trim-to-MP4 workflow are implemented. Their verification varies by platform: the
> end-to-end capture pipeline is tested on Linux/X11, while Windows capture only compiles and has
> not yet run on Windows hardware. See [`ROADMAP.md`](ROADMAP.md) and
> [`docs/gaps.md`](docs/gaps.md) for current evidence and limits.

ClipCraft takes inspiration from Medal's capture-first workflow and compact dark interface. It
keeps the focus on recording, finding, playing, trimming, and exporting local clips. It leaves out
feeds, social features, accounts, cloud sharing, and a general-purpose video editor.

## What works now

- **Capture and replay:** choose a screen or window, record into a bounded replay buffer, and save
  clips with the global hotkey. Recording supports MP4/MKV, configurable quality and framing,
  replay length, post-roll, and storage limits. Linux/X11 has end-to-end synthetic-screen evidence;
  Windows Graphics Capture currently compiles but has not been run on Windows hardware. Game-only
  mode is on by default: replay stays stopped until a user-added game process and capturable window
  are found, then stops when that process exits. Manual capture mode is available by turning the
  setting off.
- **Audio:** synchronized system audio and microphone recording, mixed or on separate AAC tracks,
  with levels, mute, push-to-talk, and device-loss handling. The recorder path is tested with
  generated sources; real microphone and speaker hardware has not been tested here.
- **Playback:** open local clips in the app with sound, seek and step frames, adjust volume and
  playback speed, loop, select an audio track, and use fullscreen. Clip thumbnails are decoded and
  cached locally.
- **Library:** keep a rebuildable local index, restore the last folder, scan and search clips,
  sort them, view media details, reveal or relink files, and organize clips with favorites,
  collections, and game labels. Library labels are inferred from some filenames or set by hand;
  Game-only capture does not yet tag clips in the library. Folder scanning is non-recursive.
- **Quick trim and export:** mark a range in the player and save an original-quality trim, or
  choose an estimated 10 MB, 25 MB, or custom 1–500 MB small-file export. Compression uses the Rust
  software H.264 encoder, preserves AAC audio tracks, and reduces video to at most 720p/30 fps
  without upscaling. Both paths preserve the source and support progress/cancel; compressed exports
  report their actual size. No upload or Discord account is involved.
- **Settings and diagnostics:** recording, quality, audio, hotkey, storage, game allowlist, and
  general settings are stored locally. Game rules can match executable-name substrings and exact
  Steam app IDs on Linux. The app and CLI expose local diagnostics; the CLI can also list capture
  targets, record for a set duration, inspect the library, and trim clips.

## Known platform and format limits

- Linux/X11 capture and replay have been tested end to end in a virtual display. Wayland support is
  partly verified; no real desktop portal chooser has delivered a frame yet.
- The Windows capture provider and Windows hotkeys compile, but have not been run on Windows.
  macOS capture is not implemented.
- Game-only capture is the default and stays idle until a game is added to the allowlist. X11 and
  Windows window-to-process adapters are implemented; this environment verifies X11 with a
  synthetic window, not a real game. Wayland does not expose the
  process-to-window mapping used by Game source, and macOS capture is not available.
- Hardware encoders are connected but have only been exercised against stand-ins in this
  environment, not a real GPU. The recorder now has an optional opaque native-surface handoff and
  CPU-readback fallback, both tested with stand-ins, but no platform capture/encoder uses that route
  yet. Frames still pass through CPU-owned BGRA memory and CPU-side conversion before encoding, so
  this is not yet a low-copy, GPU-resident capture-to-encode path. The software encoder is not fast
  enough for reliable 1080p60 here; performance on a gaming PC still needs measurement. Vice's
  reported sub-1% CPU use is not a ClipCraft performance claim until measured on named hardware.
- Playback supports more formats than trim/export. Original-quality trim copies samples to MP4;
  small-file export currently accepts H.264/AAC MP4/MOV/MKV, up to 30 minutes per job, and writes
  H.264 MP4. Target sizes are estimates, not guarantees about a changing Discord upload limit.
  Export needs an output filesystem with hard-link support for safe publication; software export
  speed on gaming PCs is unmeasured. H.265 and WebM recording remain disabled.
- The library's game grouping remains filename-based or manual; recursive scans and automatic
  capture-game tagging remain open. There is no upload or link workflow.
- Real microphone/speaker capture, Windows device behavior, a real-game performance test, and the
  release soak remain open. The ranked list is in [`docs/gaps.md`](docs/gaps.md).

## Planned direction (not built)

The next product plan prioritizes Windows/Linux/macOS game-only capture evidence and a low-copy
GPU capture-to-encode path, then an Android feasibility study. Later proposals include opt-in
client-side encrypted backups to a user-selected provider, controller shortcuts, Discord Rich Presence,
keyboard/mouse overlays, safer themes/plugins, a focused one-track editor, and a local CLI/MCP
interface for footage management. These are not available in this build. See the
[`roadmap`](ROADMAP.md) and [`backlog`](docs/backlog.md) for their requirements and limitations.

ClipCraft already records video plus system/microphone audio. Voice/video calling, social feeds,
sharing links, and a community layer are not planned.

## Rust and local-data design

The desktop app, UI, engine, capture, playback, recording, CLI, and required tooling are Rust
workspace crates. ClipCraft also uses pinned open-source FilmCraft crates for time and media
components; the dependency and license decisions are recorded in
[`docs/adr/0004-filmcraft-crates.md`](docs/adr/0004-filmcraft-crates.md). This is an independent
Rust product that reuses reviewed open-source dependencies, not a formal clean-room implementation.
Two small source-to-Rust ports from Vice and ClipRecorder are included under permission reported by
the maintainer; their source files and boundaries are in [`docs/upstream-ports.md`](docs/upstream-ports.md).
GPU Screen Recorder is a capture-behavior reference only. ClipCraft plans an independent Rust
implementation against public native APIs; its GPL-3.0-only source/runtime is not translated,
shipped, or used as a runtime dependency. Native operating-system APIs sit behind Rust platform
adapters, and the current CPU-frame route remains the fallback.
FFmpeg/ffprobe are used only as external test tools, never linked or shipped as runtime dependencies.

Captured media, settings, thumbnails, and the library index stay on the device. Source clips are not
overwritten by trim/export. Users share normal exported files with whatever software they choose.

## Build, run, and test

Install the pinned Rust toolchain with `rustfmt` and `clippy`, then install the license checker:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo xtask ci
cargo run --package clipcraft
```

Useful local CLI commands:

```sh
cargo run --package clipcraft-cli -- help
cargo run --package clipcraft-cli -- capture targets
cargo run --package clipcraft-cli -- capture record --seconds 30
cargo run --package clipcraft-cli -- library scan /path/to/clips
cargo run --package clipcraft-cli -- library probe /path/to/clip.mp4
cargo run --package clipcraft-cli -- clip trim input.mp4 output.mp4 2 8
cargo run --package clipcraft-cli -- clip export input.mp4 smaller.mp4 2 8 10
cargo run --package clipcraft-cli -- diagnose
```

The desktop app needs a graphical session and an available capture provider for recording. See
[`docs/testing.md`](docs/testing.md) for focused test commands and platform evidence requirements.
For compression, the last CLI argument is the estimated target in decimal MB. In the player, open
Trim (T), choose an Export option, and save the new file beside the original. Output can be smaller
or larger than the estimate; the completion message and CLI JSON report its actual bytes.

## Project docs

- [`ROADMAP.md`](ROADMAP.md) — milestone status and release gates.
- [`PLAN.md`](PLAN.md) — product direction, architecture choices, and intended MVP.
- [`docs/gaps.md`](docs/gaps.md) — ranked limitations and the evidence behind each status.
- [`docs/backlog.md`](docs/backlog.md) — stable work IDs, requirements, and acceptance evidence.
- [`AGENTS.md`](AGENTS.md), [`CLAUDE.md`](CLAUDE.md), and [`CONTRIBUTING.md`](CONTRIBUTING.md) —
  contributor rules and workflow.
- [`docs/architecture.md`](docs/architecture.md), [`docs/testing.md`](docs/testing.md), and
  [`docs/issue-requirements.md`](docs/issue-requirements.md) — implementation boundaries and
  verification requirements.
- [`ATTRIBUTION.md`](ATTRIBUTION.md) — shipped assets and licenses.
- [`docs/upstream-ports.md`](docs/upstream-ports.md) — the scope, source, and revisions of Rust ports from reference projects.

Documentation is part of every implementation change. The PR gate requires changes under `apps/`,
`crates/`, or `tools/` to update both this README and [`docs/gaps.md`](docs/gaps.md); update the
roadmap, backlog, architecture, and testing docs when their status or guidance changes.
