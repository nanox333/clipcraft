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
not a formal clean-room implementation. The small editor additionally consumes pinned
`filmcraft-edit` and transient `filmcraft-project` track types, without adopting a project-file workflow.

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
- **Settings disclosure:** General's final `show_advanced` switch reveals bottom sections under
  an **Advanced settings** heading in Games (custom rules), Capture (framing), Quality (encoder
  effort/keyframes) and Audio (sample rate/channels). Navigation stays fixed; hidden values and
  rules are preserved. Categories retain independent scroll state. Custom-game forms share a flat
  layout with padded, responsive fields and no enclosing card; preferences still use the shared
  engine save command. See [settings.md](settings.md).
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
  CPU readback. Windows SDR WGC textures now use bounded owned D3D11 leases; compatible Media
  Foundation encoders convert to NV12 on the same GPU with a bounded sample allocator. HDR,
  scaling/cropping and incompatible encoders retain CPU fallback. OS APIs and device handles stay
  inside `clipcraft-platform`; see [ADR 0014](adr/0014-windows-d3d11-capture-encode.md) and
  [measured limitations](windows-performance.md). The 1080p60 performance gate remains open.
- **Platform:** display/window capture, audio devices, global hotkeys, and hardware encoder adapters.
  Expose safe, fallible Rust APIs. Any OS FFI is isolated here, reviewed by ADR, and documented with
  safety invariants. `sysinfo` provides Rust process enumeration; the game watcher retains only
  executable basenames, process IDs for window lookup, and (when configured) numeric Steam IDs.
- **Media and library:** video files remain normal local media files. A rebuildable, versioned local
  index stores small library metadata such as title, tags, favorites, and thumbnail paths. Caches
  and replay segments are disposable app data. There is no user-facing editor-project format in
  the MVP. Library traversal is bounded to eight child levels, 20,000 entries and 5,000 videos,
  skipping symlinks. One desktop worker accepts one replaceable pending scan and supports cooperative
  cancellation; completed scans merge on the UI thread through `library.store_scan`, preserving
  edits made while traversing. CLI `library.scan` / `library.refresh` use the same scanner.
  `library.register_recording` stores `recorded_game` separately from the manual game override;
  `SaveJob` snapshots this label at Save time, including post-roll. Only successful saves register
  metadata, and metadata errors cannot undo the saved media. The optional index field defaults to
  absent for existing version-1 indexes. Game priority is manual, recorded, folder, filename.
- **Replay:** a bounded encoded-segment ring with explicit duration/byte limits. Saving snapshots
  segment references atomically, handles post-roll, and reports any required keyframe re-encode.
- **Edit/export:** `clipcraft-export` offers sample-copy trim (`clip.trim`) and software H.264
  compression (`clip.export`). Size presets reserve the exact copied AAC bytes, estimate a video
  bitrate with overhead headroom, and choose bounded dimensions/FPS without upscaling. Frames are
  decoded, resized, encoded, and muxed sequentially; no whole-clip raw-frame buffer is built. The
  original is preserved, progress/cancel is shared with the UI, and actual output size is reported.
  Compression uses exclusive temporary-file creation and publishes via a no-replace hard link;
  the output filesystem must support hard links. See [ADR 0013](adr/0013-local-compressed-export.md).
  The same crate writes looping GIFs (`clip.export_gif`) and evens out loudness by decoding, scaling and re-encoding only the sound
  (`clip.normalize`); see [ADR 0018](adr/0018-gif-and-loudness-export.md).
  `editor.export` renders a bounded ordered range list through FilmCraft H.264/AAC encoders,
  using one source/cache at a time. One continuous audio encoder avoids per-cut priming gaps;
  the first source audio track is normalized to 48 kHz stereo. No multitrack compositor or
  editor-project file is required.
- **Engine:** owns recording sessions, input validation, local index operations, crash recovery,
  background jobs, and stable typed commands. UI and CLI dispatch these same commands. It does not
  provide a persistent editor project or autosaved edit sessions. The new bounded `editor.edit`
  session owns exact source ranges, sequence order and 64-step undo/redo; FilmCraft track/cut
  algebra is adapted behind this small public command model (ADR 0017).
- **Game-aware capture:** the engine owns matching, desktop policy and monotonic timer math;
  platform adapters expose process metadata and verified game windows. One desktop worker owns
  the reusable process observer and a bounded local Steam catalog, reads VDF/ACF files through
  `recording.scan_steam_games`, and returns snapshots via one replaceable pending request. A
  settings-key change drops obsolete result receivers. Worker failures are errors, never empty
  game-exit snapshots. UI-driven running-app refresh uses the same worker.
  Custom rules override automatic matches; Steam exclusions, built-in exclusions and global-off
  precede local-manifest fallback. Linux Steam environment IDs or paths inside canonical installed
  game directories identify running Steam games; full process paths are not retained.
  Games-only remains the default. Opt-in policies continue desktop replay for a bounded duration
  after actual game exit, or maintain desktop replay between games including startup. Games always
  select their verified window; desktop policy selects an exposed monitor and skips portal chooser
  targets. A new game cancels the desktop timer; its next exit starts another. Stop pauses desktop
  policies until Start. Desktop-policy source changes/timeout preserve the final replay through one active save
  and one queued job; a full queue halts frame production while retaining the bounded buffer.
  Output names avoid in-flight collisions and remain exclusively written. Desktop clips do not
  inherit the previous game's recorded label. See [ADR 0016](adr/0016-background-steam-and-exit-fallback.md),
  [settings.md](settings.md) and [game-catalog.md](game-catalog.md).
  Wayland automatic game-window matching and macOS capture remain unsupported; Windows/live-game
  runtime evidence is still required.
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
non-Rust runtime or upstream UI, app or artwork. Vice identification metadata is the explicit
bundled-data exception recorded in the provenance guide.

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

## Small editor workspace

The Editor rail destination owns an in-memory media pool independent of the Clips library.
`editor.import` probes local references on one bounded worker; Send to editor adds a reference
without placing it on the sequence. `editor.edit` validates integer-tick cuts and keeps at most
64 history snapshots within 8 MiB, plus a 64 MiB current-metadata budget. Duration/minimum-frame
bounds replace an arbitrary segment-count cap. UI and CLI use these same commands.
The editor uses the shared `kit` palette, Inter families and standard keyboard-aware egui buttons.
A single thumbnail response handles click, drag and context menus to avoid competing hit targets.
Dense fit views aggregate subpixel cuts, cull offscreen widgets and bound thumbnail painting;
zooming or ruler seeking still resolves the exact sequence range.
