# ClipCraft

**A local-first Rust desktop app for instant game clips: record a replay buffer, save the moment
with a hotkey, then play, search, trim and export your clips.** It works fully offline. There is no
account, server, telemetry, upload or share-link service, and everything you record stays on your
computer.

> **Status: early release (0.2.0).** Windows (tested on AMD) and Linux/X11 are supported; macOS is
> not. See [`docs/gaps.md`](docs/gaps.md) for current evidence and limits.

## Download

On Windows, download **ClipCraft-Setup** from the
[latest release](https://github.com/nanox333/clipcraft/releases), run it, and ClipCraft is installed
for your user with a Start menu shortcut and an uninstaller (Windows Settings → Apps). No
administrator rights or account are needed. Linux gets a tarball with the app. The command line tool
`clipcraft-cli` is a separate download for people who want it; the app does not need it.

Check a download against `SHA256SUMS.txt` if you like. Downloads are not code-signed, so Windows
SmartScreen may warn: choose *More info*, then *Run anyway*. ClipCraft can tell you when a newer
version is published and install it for you (Settings → About); turn the automatic check off under
Settings → General and it never goes online. See [`CHANGELOG.md`](CHANGELOG.md) for each release.

New profiles get a short setup for storage, capture, audio and shortcuts. Each hotkey action
supports up to five shortcuts: use **+** in Settings → Hotkeys.

## What it does

- **Record and replay:** capture a screen or game window into a bounded replay buffer and save the
  last moments with a global hotkey. Game-only mode waits until a recognized game is running.
- **Encoding:** hardware H.264 and H.265 on supported GPUs (Windows), or a built-in software H.264
  encoder tuned for screen recording. If the graphics-card encoder fails or is busy, recording
  continues on the processor and the card is tried again every 20 s or longer.
- **Audio:** system sound and microphone, mixed or on separate tracks, with optional offline
  microphone noise suppression and a mic test.
- **Library:** a searchable local clip library with favorites, collections and game labels. Delete
  clips to the Recycle Bin from a right-click menu.
- **Playback and editing:** smooth playback, quick trims, small-file and GIF export, loudness
  evening, and a small sequence editor with text, fades and colour effects.
- **Desktop:** searchable, categorized Settings; system tray (Windows); a "clip saved" pop-up that
  never takes focus from your game; optional Discord Rich Presence.
- **Command line:** `clipcraft-cli` runs the same commands as the app.

Details and known limits for every feature are in [`docs/features.md`](docs/features.md).

## Planned, not built

Windows/Linux/macOS game-only capture evidence, a low-copy GPU capture path, opt-in client-side
encrypted backups, controller shortcuts and overlays. Voice/video calls, social feeds and sharing
links are not planned. See the [roadmap](ROADMAP.md) and [backlog](docs/backlog.md).

## Privacy and design

The app, engine, capture, playback, CLI and tooling are Rust workspace crates, using pinned
open-source [FilmCraft](docs/adr/0004-filmcraft-crates.md) crates for media. Captured media,
settings, thumbnails and the library index stay on the device, and source clips are never
overwritten by trim or export. FFmpeg is used only as an external test tool. Source boundaries are
in [`docs/upstream-ports.md`](docs/upstream-ports.md).

## Build, run, and test

Install the pinned Rust toolchain with `rustfmt` and `clippy`, then install the license checker:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo xtask ci
cargo run --package clipcraft
```

`cargo xtask ci` runs the same checks as GitHub Actions on Linux and Windows; tests clean their scratch
folders with a retry so a finishing capture worker cannot make them flaky.

Useful local CLI commands:

```sh
cargo run --package clipcraft-cli -- help
cargo run --package clipcraft-cli -- capture targets
cargo run --package clipcraft-cli -- capture record --seconds 30
cargo run --package clipcraft-cli -- library scan /path/to/clips
cargo run --package clipcraft-cli -- library probe /path/to/clip.mp4
cargo run --package clipcraft-cli -- clip trim input.mp4 output.mp4 2 8
cargo run --package clipcraft-cli -- clip export input.mp4 smaller.mp4 2 8 10
cargo run --package clipcraft-cli -- clip gif input.mp4 clip.gif 2 8
cargo run --package clipcraft-cli -- clip normalize input.mp4 even.mp4 0 30 -16
cargo run --package clipcraft-cli -- diagnose
```

The desktop app needs a graphical session and an available capture provider for recording. See
[`docs/testing.md`](docs/testing.md) for focused test commands and platform evidence requirements.
For compression, the last CLI argument is the estimated target in decimal MB. In the player, open
Trim (T), choose an Export option, and save the new file beside the original. Output can be smaller
or larger than the estimate; the completion message and CLI JSON report its actual bytes.

## Project docs

- [`CHANGELOG.md`](CHANGELOG.md) — what each release contains.
- [`docs/features.md`](docs/features.md) — detailed features and platform limits.
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
