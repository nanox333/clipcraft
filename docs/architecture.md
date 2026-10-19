# Architecture

ClipCraft is a **Rust-only native desktop application**. Every first-party runtime component is a
Rust crate in the Cargo workspace. The supported product has no backend, web client, account system,
network endpoint, or server process. Capture, library, edit, and export must work offline.

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

The implemented workspace currently contains `clipcraft-time` and `clipcraft-frame` (L0),
`clipcraft-media` (L1), `clipcraft-player` (L2), `clipcraft-engine` (L4), `clipcraft-audio-out` (L5), and the `clipcraft`, `clipcraft-cli`, and `xtask`
entry points (L6). Capture, media decoding, edit, and export crates remain planned. FilmCraft crates
are consumed as pinned git dependencies ([ADR 0004](adr/0004-filmcraft-crates.md)).

- **Time:** `clipcraft-time` re-exports FilmCraft's exact `Tick` and `FrameRate` and adds validated
  constructors and audio sample positions. Floating-point seconds are display-only.
- **Frame queues:** `clipcraft-frame` provides item- and byte-bounded queues with observable drops.
- **Playback:** `clipcraft-player` decodes a clip on a worker thread against a wall clock and hands
  RGBA frames to the UI. It reads files in place, skips frames when decoding falls behind, applies
  only the latest of several queued seeks, and reports a decoder panic as a failure.
- **Sound:** `clipcraft-player::audio` decodes a clip's audio at the output device's rate into a
  bounded feed. The device callback (`clipcraft-audio-out`, the only crate that talks to the OS
  audio system) drains it and counts only frames it really delivered; that count is the playback
  clock, so the picture follows the sound and an underrun pauses the picture instead of desyncing
  it. With no device, or after a device error, playback keeps time with the wall clock, and the app
  restarts a failed device a few times.
- **Thumbnails:** `clipcraft-player::thumbnail` decodes a frame about a tenth into the clip, reduces it
  in the decoder's native format before converting to RGBA, and stores a 320 px JPEG in a bounded,
  atomically written cache. The app creates textures only for visible cards, a few per frame.
- **Media probing:** `clipcraft-media` reads MP4/MOV container headers (duration, size, frame rate,
  codecs) after bounding header sizes; it never decodes picture or sound data.
- **Capture model:** portable source descriptors, capabilities, packets, timestamps, and failure
  states. No OS APIs or device handles leak into this layer.
- **Platform:** display/window capture, audio devices, global hotkeys, and hardware encoder adapters.
  Expose safe, fallible Rust APIs. Any OS FFI is isolated here, reviewed by ADR, and documented with
  safety invariants.
- **Media and library:** video files remain normal local media files. A rebuildable, versioned local
  index stores small library metadata such as title, tags, favorites, and thumbnail paths. Caches
  and replay segments are disposable app data. There is no user-facing editor-project format in
  the MVP.
- **Replay:** a bounded encoded-segment ring with explicit duration/byte limits. Saving snapshots
  segment references atomically, handles post-roll, and reports any required keyframe re-encode.
- **Edit/export:** a small in-memory cut/trim model and local export jobs with progress,
  cancellation, and actionable error states. The first release does not need a multitrack timeline,
  compositor, or editor-project file.
- **Engine:** owns session, validation, undo/redo, autosave/recovery, background jobs, and stable
  typed commands. UI and CLI dispatch these same commands.
- **UI/CLI:** native egui desktop app and local command-line tool. No HTTP/WebSocket control service,
  MCP server, browser UI, or remotely hosted feature is part of this architecture.
- **Current shell limit:** the UI scans a user-selected folder and displays file metadata. It does not
  decode video or start capture; unavailable actions are disabled or labeled.

## Rust and dependency policy

Use Rust for first-party code and required tooling. Do not add JavaScript/TypeScript, Node, C/C++
product code, Electron, Tauri, or FFmpeg to the application. Select permissively licensed Rust
dependencies and record licenses. FFmpeg/ffprobe are permitted only as external test oracles or
fixture generators; they are never linked or shipped.

The platform boundary may call native operating-system APIs through Rust bindings. It must report
capability failures cleanly and keep portable crates buildable without the OS-specific provider.
Every crate declares its layer; `cargo xtask layers` checks for invalid dependencies.

## Local data and privacy

The app stores the configured replay buffer, clips, library index, thumbnails, and preferences on
the user's device. Every queue and cache has a documented bound. Export never overwrites its input.
There is no network path for captured media.

See [`PLAN.md`](../PLAN.md) for the proposed crate list, capture design, and milestones. Changes to
the library metadata schema or platform safety boundary require an ADR.
