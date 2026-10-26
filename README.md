# ClipCraft

ClipCraft's **CUT** identity uses an original lime C/play mark in the native window, header and
Discord product badge. [Brand guide and reproducible asset kit](docs/brand.md).

**A local-first Rust desktop app for instant game clips, playback, a searchable clip library, and
quick trims and small clip sequences.** The current build captures and manages clips offline; it has no account, hosted
backend, backup, upload, sync, or share-link service.

> **Status: active prototype.** The screen-capture/replay/save pipeline, playback, local library,
> and basic trim-to-MP4 workflow are implemented. Their verification varies by platform: the
> end-to-end capture pipeline is tested on Linux/X11 and with a generated window on Windows/AMD.
> Windows/AMD has a two-hour generated-window 1080p30 soak and one-minute native SDR 1080p60
> throughput evidence; real games and a two-hour 1080p60 soak remain open.
> See [`ROADMAP.md`](ROADMAP.md) and
> [`docs/gaps.md`](docs/gaps.md) for current evidence and limits.

ClipCraft takes inspiration from Medal's capture-first workflow and compact dark interface. It
keeps the focus on recording, finding, playing, trimming, and exporting local clips. It leaves out
feeds, social features, accounts, cloud sharing, and a general-purpose video editor.

## What works now

- **System tray (Windows):** a custom-styled tray menu (status, start/stop recording, save clip, screenshot, clips folder, library, settings, quit) and, by default, closing the window
  minimizes it so capture keeps running (Settings → General → Keep running in the tray). The taskbar
  button stays; a fully hidden window would stop capture.
- **Capture and replay:** choose a screen or window, record into a bounded replay buffer, and save
  clips with the global hotkey. Recording supports MP4/MKV, configurable quality and framing,
  replay length, post-roll, and storage limits. Linux/X11 has end-to-end synthetic-screen evidence;
  Windows/AMD has generated-window capture-to-MP4 evidence, including a native D3D11 path. Game-only
  mode is on by default: replay stays stopped until a recognized game process and capturable window
  are found, then stops when that process exits. Games settings also offer a timed desktop replay
  after a game closes (15 seconds–1 hour), or continuous desktop replay that automatically switches
  to game windows and back. Reopening a game cancels the timer; its next exit starts a fresh timer.
  Automatic source changes in the desktop modes save the final replay; buffers remain bounded.
  Manual capture is also available.
- **Audio:** synchronized system audio and microphone recording, mixed or on separate AAC tracks,
  with levels, mute, push-to-talk, and device-loss handling. The recorder path is tested with
  generated sources; real microphone and speaker hardware has not been tested here.
- **Playback:** open local clips in the app with sound, seek and step frames, adjust volume and
  playback speed, loop, select an audio track, and use fullscreen. Clip thumbnails are decoded and
  cached locally. Playback retains only its newest pending picture when the UI stalls; new seeks
  discard older pending/in-flight pictures rather than uploading a backlog (M11.1f). Queued playback
  and sound decoding now get priority over background thumbnails and waveforms (M11.1g). Rapid
  percentage-seek shortcuts keep the last key you pressed, including within one UI frame.
- **Library:** keep a rebuildable local index, restore the last folder, scan and search clips,
  sort them, view media details, reveal or relink files, and organize clips with favorites,
  collections, and game labels. Scans include subfolders (eight levels), skip symlinks, and run on
  a cancellable desktop worker; the CLI uses the same bounded traversal. Game-managed saves remember
  the game active when Save was pressed. Manual labels override that captured game, followed by
  folder/filename guesses; labels survive rescans and restarts in the local index.
  Search/sort results and sidebar counts are cached; the grid builds visible rows and supports
  keyboard navigation across offscreen clips. Static keyboard focus no longer keeps redrawing.
  Index writes run on one bounded background worker, with ordered edits and retry on failure.
  Closing waits for accepted writes; the app warns before discarding unsaved metadata. Abrupt
  termination can lose unacknowledged metadata; source clips remain untouched.
- **Quick trim and export:** mark a range in the player and save an original-quality trim, or
  choose an estimated 10 MB, 25 MB, or custom 1–500 MB small-file export. Compression uses the Rust
  software H.264 encoder, preserves AAC audio tracks, and reduces video to at most 720p/30 fps
  without upscaling. Both paths preserve the source and support progress/cancel; compressed exports
  report their actual size. Both writers use exclusive temporary files and no-replace publication.
  Their output filesystem must support hard links. Automatic copy names skip occupied partial/recovery
  files. When recording is set to the GPU encoder, small-file export tries it
  first, and the GPU decoder too, falling back to software on any trouble (untested on a real GPU
  so far). No upload or Discord account is involved.
- **GIF and loudness:** the same Trim panel can save a range of up to 30 seconds as a looping GIF
  (480 px wide, 12 fps, no sound) or even out its loudness to -16 LUFS (the picture is copied, the
  sound is re-encoded, and the result is never pushed into clipping). See
  [ADR 0018](docs/adr/0018-gif-and-loudness-export.md); platform evidence is limited to Linux
  generated clips. GIF frame delays use cumulative centisecond timestamps so
  rates such as the default 12 fps preserve the requested duration.
  Joining clips is done in the small editor below.
- **Small sequence editor:** open its own **Editor** rail icon, **Import clips**, or **Send to editor**
  from Clips. Its resizable Library/Effects/Text panel, large preview and bottom timeline share the
  app's charcoal/lime styling, SVG icons (`assets/icons`) and embedded Inter typefaces. Append, split, trim, reorder, duplicate
  or remove cuts, with 64-step undo/redo, linked audio volume/mute, black/white fades and seven colour
  effects. Independent text layers can be moved, trimmed, split, duplicated, faded and recoloured.
  Preview and export share FilmCraft text rendering and the same audio fade envelope.
  They now also share a bounded compositor for aspect-fit scaling, colour effects, fades and text
  (M11.1i): compatible hardware uses wgpu, with automatic CPU fallback. Paused edits refresh on
  the decode worker; portrait/ultrawide sources keep their aspect ratio. Decode/font work and
  pixel upload/readback remain; physical-GPU speed and Metal/DX12 runtime are not verified here.
  The UI keeps only selected-text handle geometry; it no longer duplicates text textures or effects.
  Editor import resolves source bounds with the playback reader, so AAC padding cannot make a
  freshly captured clip's full-length timeline or export fail. Send to editor resolves these facts
  in the background before the clip can be added.
  Timeline strips sample the source pictures and linked audio waveforms; bounded background
  analysis keeps media work off the UI thread. Preview prepares the next source across cuts.
  **Export video** offers standard H.264 MP4 or estimated 10 MB, 25 MB and custom 1–500 MB presets,
  with actual size and encoder reporting, progress/cancel and GPU-encoder fallback to software.
  Choose **720p**, **1080p**, or **Source (up to 4K)** at **30 or 60 FPS**, with stereo AAC.
  Size presets can reduce resolution or bitrate; the dialog explains reductions and shows actual
  output settings. Source mode preserves the first clip's portrait or ultrawide framing.
  Exports run in the background with keyboard play/pause and frame stepping still available; progress
  and cancellation stay in the top toolbar, including in shorter windows. Cancellation shows its
  pending state until the worker stops. Matching-size exports skip redundant frame copies.
  Repeatable software-export measurements and their limits are in [the performance review](docs/performance.md).
  Edits recover from internal local metadata after restart, including text, export preset and quality;
  undo history starts fresh. A restored draft never takes over launch: ClipCraft opens on Clips and
  the Editor tab reopens it. In short windows the editor page scrolls so the preview keeps its
  height. Originals remain untouched, and quick trim stays available. See
  [the editor guide](docs/editor.md) for controls and limits.
- **Local match data:** optional CS2/Dota 2 GSI adapters and a League of Legends local API reader
  show selected map/match/player details in Settings → Games. All default off; Valve config setup
  preserves existing files. Snapshots are transient and are not stored with clips. Discord
  disclosure is controlled separately below.
  League transport requires Windows/macOS. Protocol/socket tests and Windows compilation are
  verified; live matches/native League TLS still need target-OS checks. See [setup and limits](docs/game-integrations.md).
- **Discord Rich Presence:** opt-in activity, with separate game-name and selected match-context
  switches in General. A bounded local IPC worker handles update/clear/reconnect independently
  of capture. No media, paths, clip titles or account identifiers are published. Automatic setup uses a ClipCraft
  build with the shared ClipCraft identity configured once by the maintainer; end users then only
  enable the toggle. A Medal-inspired local card preview works without an ID, with original badges
  and optional readable match details. With game-name consent, CS2, Dota 2 and League use
  Discord-hosted game icons with the small CUT badge; publisher image files are not bundled.
  ClipCraft makes no artwork web requests, and its offline preview uses CUT. The shared public application ID is now included in ordinary
  builds. The CUT application icon is uploaded; real Windows profile checks show game artwork
  and the small CUT badge after Discord activity sharing is enabled. Other-account audience
  checks remain open. [Setup, exact fields and limits](docs/discord-presence.md).
- **Settings and diagnostics:** recording, quality, audio, hotkey, storage, game allowlist, and
  general settings are stored locally. [348 known games](docs/game-catalog.md) are recognized
  automatically, with per-game exclusions and a global switch in Settings › Games. The Settings
  Overview summarizes the setup and suggests fixes; the advanced toggle in General reveals the custom-game forms. The [custom-game form](docs/settings.md) is a card like every other settings section, with padded,
  responsive fields and is advanced-only. Custom rules take priority over the built-in catalog and can match executable-name substrings and exact
  Steam IDs. A bounded background worker reads processes and [local Steam manifests](docs/game-catalog.md)
  to recognize additional installed games without a download; custom rules and exclusions take priority.
  The app and CLI expose local diagnostics; the CLI can also list capture
  targets, record for a set duration, inspect the library, and trim clips.

## Known platform and format limits

Replay snapshots share immutable encoded-video bytes with the save job, avoiding a full-buffer
copy while recording. Replay insertion caches keyframe boundaries, and full-frame RGBA conversion
reuses output storage without redundant clears. Repeatable synthetic CPU measurements and remaining
GPU/capture limitations are documented in the [performance review](docs/performance.md). Run
`cargo run --release -p clipcraft-record --example performance` to reproduce the CPU benchmarks;
they do not establish game-FPS impact or sustained capture performance.

- Linux/X11 capture and replay have been tested end to end in a virtual display. Wayland support is
  partly verified; no real desktop portal chooser has delivered a frame yet.
- Windows/AMD native SDR capture avoids full-frame CPU readback at matching source/output size.
  Nonblocking latest-frame reads and a bounded SDR update-interval request have one-minute
  generated-window 1080p60 evidence, with fresh captures counted separately from repeated images.
  Standard-colour (SDR) capture is the default; the optional HDR compatibility setting retains CPU
  readback. A source of another size than the clip (scaling, cropping or letterboxing) is now
  scaled by the graphics card with black bars, when the encoder supports GPU input; incompatible
  encoders retain the CPU fallback. The scaling is covered by a Windows GPU test and a fake-encoder
  recorder test, but has not yet been measured on real hardware. An earlier 32 fps encoder
  wait stall did not reproduce before these changes, so its cause remains unexplained.
  See [Windows measurements and reproduction](docs/windows-performance.md). A two-hour 1080p60
  soak, Windows hotkeys, real-game impact and other GPUs remain unverified; macOS capture is not
  implemented.
- Game-only capture is the default and stays idle until a recognized game is running. X11 and
  Windows window-to-process adapters are implemented; this environment verifies X11 with a
  synthetic window, not a real game. Wayland does not expose the
  process-to-window mapping used by Game source, and macOS capture is not available.
- Linux hardware encoders have only been exercised against stand-ins here. Windows/AMD has the
  native capture-to-encode evidence described above, with CPU fallback for incompatible settings.
  Linux frames still pass through CPU-owned memory and conversion. Performance on gaming PCs and
  other GPUs needs measurement; Vice's reported sub-1% CPU use is not a ClipCraft performance claim.
- Playback supports more formats than trim/export. Original-quality trim copies samples to MP4;
  small-file export currently accepts H.264/AAC MP4/MOV/MKV, up to 30 minutes per job, and writes
  H.264 MP4. Target sizes are estimates, not guarantees about a changing Discord upload limit.
  Export needs an output filesystem with hard-link support for safe publication; software export
  speed on gaming PCs is unmeasured. WebM recording remains disabled; H.265 recording needs the graphics card (Windows).
- Game labels come from manual overrides, captured-game metadata or folder/filename guesses.
  Recognition does not retroactively label existing videos. There is no upload or link workflow.
- Real microphone/speaker capture, Windows device behavior, a real-game performance test, and the
  remaining release checks stay open. The ranked list is in [`docs/gaps.md`](docs/gaps.md).

## Planned direction (not built)

The next product plan prioritizes Windows/Linux/macOS game-only capture evidence and a low-copy
GPU capture-to-encode path, then an Android feasibility study. Later proposals include opt-in
client-side encrypted backups to a user-selected provider, controller shortcuts,
keyboard/mouse overlays, safer themes/plugins, further editor evaluation, and a local CLI/MCP
interface for footage management. These are not available in this build. See the
[`roadmap`](ROADMAP.md) and [`backlog`](docs/backlog.md) for their requirements and limitations.

ClipCraft already records video plus system/microphone audio. Voice/video calling, social feeds,
sharing links, and a community layer are not planned. The small editor includes cuts, arrangement,
text layers, colour effects and simple fades. Overlapping video, cross-dissolves, detached audio
and general-purpose multitrack editing remain outside its scope.

## Rust and local-data design

The desktop app, UI, engine, capture, playback, recording, CLI, and required tooling are Rust
workspace crates. ClipCraft also uses pinned open-source FilmCraft crates for time and media
components; the dependency and license decisions are recorded in
[`docs/adr/0004-filmcraft-crates.md`](docs/adr/0004-filmcraft-crates.md). This is an independent
Rust product that reuses reviewed open-source dependencies, not a formal clean-room implementation.
Selected Vice matching code is included under permission reported by the maintainer. The dormant
ClipRecorder audio utility was removed in M11.1j because no recording path used it. Current and historical
source boundaries are in [`docs/upstream-ports.md`](docs/upstream-ports.md).
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
