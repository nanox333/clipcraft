# ClipCraft

**A Rust-only desktop app for instant game clips. All capture, editing, library, and export features
run locally. No hosted server, account, upload, sync, or web app is required.**

> **Status: early build.** The Rust workspace, local library scanner, CLI, and native desktop shell
> are in place. Capture, video playback, persistent library metadata, editing, and export are not
> implemented yet.

ClipCraft is inspired by Medal's capture-first workflow and dark compact interface. It aims to keep
the familiar game/status strip, dense thumbnail library, focused editor, and local storage controls
while leaving out the feed, social graph, account system, and cloud sharing.

## Product loop

Choose a display or window, keep a bounded replay buffer, save the last configured interval with a
hotkey, make quick cuts or trims, and export a standard video file to a local folder. The exported
file is yours to share with any other software you choose.

## Rust-only implementation

All first-party product code lives in a Rust Cargo workspace: the native egui desktop UI, shared
engine commands, local media scanner, exact time primitives, and CLI. No JavaScript
or TypeScript UI, Node runtime, Electron or Tauri shell, backend, or server process is part of the
design. OS media APIs will use Rust bindings behind a small platform crate. See
[`AGENTS.md`](AGENTS.md) for the binding language and safety requirements.

The initial target is Windows 10/11. macOS and Linux capture providers follow after the local
Windows capture-to-export path is reliable. The project will use permissively licensed Rust
dependencies; FFmpeg is not a runtime dependency.

## Current scope

- **Working now:** choose or enter a local folder, scan it for video files, search titles, sort by
  name or date, open local details, copy file paths, and run the same library-scan command from the
  CLI. The current card art is an original placeholder until local video decoding and thumbnails
  are implemented.
- **Not implemented yet:** replay capture, hotkeys, video playback, persistent library metadata,
  audio capture, basic cut/trim editing, and MP4 export.
- **Explicitly excluded:** hosted services, server requirements, accounts, uploads, cloud sync,
  share links, feeds, comments, livestreaming, DRM bypass, and anti-cheat-sensitive injection.

## Documentation

- [`PLAN.md`](PLAN.md) — product, architecture, visual references, milestones, and acceptance gate.
- [`AGENTS.md`](AGENTS.md) — binding safety, Rust, licensing, privacy, and quality rules.
- [`CLAUDE.md`](CLAUDE.md) — agent session and issue workflow.
- [`ROADMAP.md`](ROADMAP.md) — current milestones and release gates.
- [`docs/backlog.md`](docs/backlog.md) — initial issue breakdown with per-issue requirements,
  limitations, and acceptance evidence.
- [`docs/issue-requirements.md`](docs/issue-requirements.md) — required issue fields and acceptance rules.
- [`docs/architecture.md`](docs/architecture.md) — planned Rust crate boundaries.
- [`docs/adr/0001-rust-only-local-product.md`](docs/adr/0001-rust-only-local-product.md) — pure-Rust and local-only product decision.
- [`docs/gaps.md`](docs/gaps.md) — ranked known gaps; evidence is required before completion claims.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`docs/testing.md`](docs/testing.md) — change and test requirements.

## Build and run

Install the Rust stable toolchain with the `rustfmt` and `clippy` components, then install the
development license checker with `cargo install cargo-deny --locked`.

```sh
cargo xtask ci
cargo run --package clipcraft
cargo run --package clipcraft-cli -- commands
cargo run --package clipcraft-cli -- library scan /path/to/video/folder
cargo run --package clipcraft-cli -- library path /path/to/video.mp4
cargo run --package clipcraft-cli -- library reveal /path/to/video.mp4
cargo run --package clipcraft-cli -- library probe /path/to/video.mp4
cargo run --package clipcraft-cli -- library thumbnail /path/to/video.mp4
cargo run --package clipcraft-audio-out --example play -- /path/to/video.mp4   # sound check, no window
```

The desktop app needs a graphical session. The scanner only reads one selected folder, does not
recurse, and caps one scan at 20,000 directory entries and 5,000 video files. The capture and editor
controls remain clearly marked unavailable until their local backends exist.
