# Architecture

ClipCraft is a **pure-Rust native desktop application**. Every first-party runtime component is a
Rust crate in the Cargo workspace. The core product has no required backend, web client, account
system, or server process. Capture, library, edit, and export must work offline. An optional
client-side-encrypted backup is planned but not implemented; its provider adapter must remain
separate from the offline core.

## Dependency layers

Dependencies flow upward only:

```text
L0  time, geometry, pixel/audio buffers, bitstream
L1  capture model, audio, media, local library
L2  cut/trim edit algebra, codecs, thumbnails
L3  local export jobs
L4  engine session, local metadata, command registry
L5  egui UI, Rust platform adapters
L6  clipcraft desktop app, clipcraft CLI, Rust xtask
```

The implemented workspace contains `clipcraft-time` and `clipcraft-frame` (L0),
`clipcraft-capture` and `clipcraft-media` (L1), `clipcraft-player` and `clipcraft-export` (L2),
`clipcraft-record` (L3), `clipcraft-engine` (L4), `clipcraft-audio-out` and
`clipcraft-platform` (L5), and the `clipcraft`, `clipcraft-cli`, and `xtask` entry points (L6).
Capture, replay recording, playback, local indexing, and trim export are implemented with the
platform evidence and limits listed in [`gaps.md`](gaps.md). FilmCraft crates are consumed as pinned
git dependencies ([ADR 0004](adr/0004-filmcraft-crates.md)); this is reviewed open-source reuse,
not a formal clean-room implementation.

- **Time:** `clipcraft-time` re-exports FilmCraft's exact `Tick` and `FrameRate` and adds validated
  constructors and audio sample positions. Floating-point seconds are display-only.
- **Frame queues:** `clipcraft-frame` provides item- and byte-bounded queues with observable drops.
- **Playback:** `clipcraft-player` decodes a clip on a worker thread against a wall clock and hands
  RGBA frames to the UI. It reads files in place, skips frames when decoding falls behind, applies
  only the latest of several queued seeks, and reports a decoder panic as a failure.
- **Typeface:** the UI uses Inter (SIL OFL 1.1) in Regular, Medium, and SemiBold, embedded from
  `assets/fonts/` and installed by `kit::install_fonts` as the proportional face plus named
  `medium` and `semibold` families (egui has no synthetic bold). egui's own fonts stay behind it
  as fallbacks for symbols and emoji. It covers Latin, Greek and Cyrillic; CJK, Arabic and Thai
  titles are not covered yet (see the shared `craft-fonts` repo for candidates).
- **Look and feel:** `apps/clipcraft/src/kit.rs` holds the palette, the egui theme, painted icons and
  shared widgets (buttons, chips, pill menus, sliders, gradients). Icons are drawn from strokes and
  polygons, not text glyphs, because egui's bundled fonts miss many symbols. Every computed width is
  clamped at zero, since egui panics on a negative one.
- **Sound:** `clipcraft-player::audio` decodes a clip's audio at the output device's rate into a
  bounded feed. The device callback (`clipcraft-audio-out`, the only crate that talks to the OS
  audio system) drains it and counts only frames it really delivered; that count is the playback
  clock, so the picture follows the sound and an underrun pauses the picture instead of desyncing
  it. With no device, after a device error, or when a device stalls silently, playback keeps time
  with the wall clock, and the app restarts a failed device a few times. Speed changes use a small
  WSOLA time-stretcher (`clipcraft-player::stretch`) so pitch is kept; 1× bypasses it.
- **Thumbnails:** `clipcraft-player::thumbnail` decodes a frame about a tenth into the clip, reduces it
  in the decoder's native format before converting to RGBA, and stores a 320 px JPEG in a bounded,
  atomically written cache. The app creates textures only for visible cards, a few per frame.
- **Media probing:** `clipcraft-media` reads supported MP4/MOV and Matroska/WebM metadata after
  bounding input sizes. Playback decoding and thumbnail generation are handled by
  `clipcraft-player` using pinned FilmCraft crates.
- **Capture model:** portable source descriptors, capabilities, packets, timestamps, and failure
  states. `CaptureSource` retains its CPU-frame method and offers an optional opaque native-surface
  lease; a compatible hardware encoder may consume that lease directly or the recorder requests a
  CPU readback. The handoff and fallback are tested with stand-ins, but no platform provider or
  encoder implements the route yet, so the active pipeline still reads frames into CPU-owned BGRA
  and converts them on the CPU. OS APIs and device handles stay inside `clipcraft-platform`; see
  M5.2 in the backlog. No GPU performance claim is supported yet.
- **Platform:** display/window capture, audio devices, global hotkeys, and hardware encoder adapters.
  Expose safe, fallible Rust APIs. Any OS FFI is isolated here, reviewed by ADR, and documented with
  safety invariants. `sysinfo` provides Rust process enumeration; the game watcher retains only
  executable basenames, process IDs for window lookup, and (when configured) numeric Steam IDs.
- **Media and library:** video files remain normal local media files. A rebuildable, versioned local
  index stores small library metadata such as title, tags, favorites, and thumbnail paths. Caches
  and replay segments are disposable app data. There is no user-facing editor-project format in
  the MVP.
- **Replay:** a bounded encoded-segment ring with explicit duration/byte limits. Saving snapshots
  segment references atomically, handles post-roll, and reports any required keyframe re-encode.
- **Edit/export:** `clipcraft-export` offers sample-copy trim (`clip.trim`) and software H.264
  compression (`clip.export`). Size presets reserve the exact copied AAC bytes, estimate a video
  bitrate with overhead headroom, and choose bounded dimensions/FPS without upscaling. Frames are
  decoded, resized, encoded, and muxed sequentially; no whole-clip raw-frame buffer is built. The
  original is preserved, progress/cancel is shared with the UI, and actual output size is reported.
  Compression uses exclusive temporary-file creation and publishes via a no-replace hard link;
  the output filesystem must support hard links. See [ADR 0013](adr/0013-local-compressed-export.md).
  No multitrack timeline, compositor, or editor-project file is required.
- **Engine:** owns recording sessions, input validation, local index operations, crash recovery,
  background jobs, and stable typed commands. UI and CLI dispatch these same commands. It does not
  yet provide a persistent editor project, autosaved edit sessions, or general undo/redo history.
- **Game-aware capture:** `clipcraft-engine` owns the user-rule matcher and deterministic arming
  state; `clipcraft-platform` observes local processes and resolves X11/Windows game windows by
  process ID. The app polls every two seconds only when Game-only mode is enabled, arms the bounded
  replay buffer for a matching process, and stops it when that process exits. Rules are local and
  user-managed; no Vice game database is included. Wayland process-to-window lookup and macOS
  capture are not supported yet, and real Windows/game evidence remains open in `gaps.md`.
- **Audio timeline policy:** `clipcraft-record::audio_timeline` aligns timestamped native audio
  packets. The current `AudioSource` mixer remains clock-driven and does not call this helper until
  a timestamped platform source is connected.
- **UI/CLI:** native egui desktop app and local command-line tool. No HTTP/WebSocket control service,
  MCP server, browser UI, or remotely hosted feature is part of this architecture.
- **Current product status:** the UI provides recording controls/settings, a persistent local clip
  library, playback, and trim. Capture/playback evidence varies by platform; see the current
  platform and performance limits in [`gaps.md`](gaps.md) rather than inferring support from code
  compiling alone.

## Rust and dependency policy

Use Rust for first-party code and required tooling. Do not add JavaScript/TypeScript, Node, C/C++
product code, Electron, Tauri, or FFmpeg to the application. Select permissively licensed Rust
dependencies and record licenses. FFmpeg/ffprobe are permitted only as external test oracles or
fixture generators; they are never linked or shipped.

Small source-to-Rust ports from Vice and ClipRecorder are tracked with their upstream revisions,
scope, and permission provenance in [`upstream-ports.md`](upstream-ports.md). Those ports add no
non-Rust runtime; they do not include either upstream UI, app, bundled game database, or assets.

The platform boundary may call native operating-system APIs through Rust bindings. It must report
capability failures cleanly and keep portable crates buildable without the OS-specific provider.
Every crate declares its layer; `cargo xtask layers` checks for invalid dependencies.

## Local data and privacy

The app stores the configured replay buffer, clips, library index, thumbnails, and preferences on
the user's device. Every queue and cache has a documented bound. Export never overwrites its input.
The current implementation has no network path for captured media. A future backup adapter may
transfer only locally encrypted data after explicit user setup; it must remain outside the offline
capture/library/edit/export path.

See [`PLAN.md`](../PLAN.md) for product direction, capture constraints, and milestones. Changes to
the library metadata schema or platform safety boundary require an ADR.
