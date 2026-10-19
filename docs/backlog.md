# Initial issue backlog

This is the issue breakdown for the early application workspace. These IDs are stable
work-item IDs; create a GitHub issue from the matching template before implementation and add its
number here. Every item inherits the constraints in [`issue-requirements.md`](issue-requirements.md).

## M0 — product and architecture

### M0.1 — Capture capability and product constraints

**Status:** documented, not measured — the platform matrix is in
[`ADR 0002`](adr/0002-capture-platform-matrix.md) and the remaining decisions are in
[`ADR 0003`](adr/0003-m0-open-decisions.md). Every capture row comes from public documentation;
reference-hardware measurements are the open evidence and belong to the M2/M3 spikes.

**Requirements:** document Windows/macOS/Linux capture and audio APIs, permissions, min OS versions,
encoder availability, and known unsupported sources; record the pure-Rust and no-hosted-server
constraints in an ADR; make `docs/gaps.md` reflect measured evidence only.

**Limitations:** no anti-cheat-sensitive injection, kernel driver, DRM bypass, livestream, account,
upload, web app, or remote-control endpoint.

**Acceptance:** the platform matrix names one supported Windows path and clear failure/fallback
behavior; all network/server requirements are absent; open decisions and their evidence are linked.

### M0.2 — Rust workspace, layering, and quality gate

**Status:** implemented; `cargo xtask ci` passes locally and CI calls the same gate on Linux and Windows runners (the Windows job is unproven until it first runs).

**Requirements:** create a Cargo workspace, `xtask`, layer checker, strict lints, license audit,
asset attribution check, and `cargo xtask ci`; make CI call the same gate; validate all crate
manifests from a clean checkout.

**Limitations:** first-party application and required tool code is Rust. No JS/TS, Node, C/C++
product component, Electron, Tauri, backend, HTTP service, or mandatory external binary runtime.

**Acceptance:** `cargo xtask ci` passes on the supported CI runner; a layer-violation fixture fails;
Rust-only and offline/no-server checks are documented and enforced where automatable.

### M0.3 — Engine command registry and local CLI skeleton

**Status:** implemented for M0 — `library.scan`, `library.path`, `library.reveal`, `commands`, and typed dispatch exist, with tests for valid, missing, and unknown parameters; capture and media commands remain future work.

**Requirements:** define stable command IDs, typed parameters, validation, error types, and JSON
stdin/stdout behavior; UI and CLI dispatch through the same registry.

**Limitations:** CLI runs locally and does not start a socket, HTTP server, MCP server, or background
daemon. No command uploads, syncs, or transmits media.

**Acceptance:** command list/help and one synthetic no-op or clip-inspection command have tests
for valid, missing, and unknown parameters; test runs need no network.

## M1 — local media and library foundation

### M1.1 — Exact time and bounded media primitives

**Status:** implemented for the current scope — integer ticks, rational frame rates, audio sample positions, and `clipcraft-frame` bounded queues (item and byte caps, drop-oldest or reject policy, every drop counted) are covered by deterministic invariant tests over common frame/sample rates, overflow, zero limits, and trim boundaries. Not yet done: frame and audio buffer types that carry media, and generated A/V fixtures beyond the timestamp test.

**Requirements:** implement integer ticks/rational rates, audio sample positions, checked conversion,
bounded frame/audio buffers, and deterministic generated fixtures.

**Limitations:** no floating-point edit-boundary math; no allocation sized directly from untrusted
input; no FFmpeg or C codec runtime.

**Acceptance:** property tests cover common integer/fractional frame rates, audio-rate conversion,
overflow, zero/invalid rates, and boundary trims; generated audio/video share a monotonic clock.

### M1.2 — Rebuildable local library index

**Status:** partial — clip duration, resolution, frame rate, and codec are probed from MP4/MOV headers on a background thread (`library.probe`, `library.store_probes`) and cached in the index until the file changes. `library.refresh` and `library.load` keep a versioned JSON index in the per-user data directory (atomic temp-file-and-rename saves, size and field caps, corrupt index rebuilt by rescan, missing files kept as offline entries, favorite/tags preserved across rescans). The desktop app restores and refreshes the last folder on launch. Library cards show real thumbnails (`library.thumbnail`): a frame about a tenth into each clip, shrunk to 320 px wide and cached as a small JPEG in the per-user data folder, keyed by path, size, and modified time, with a 5,000-file cap. A background worker fills the cache and cards load textures only while visible. UI to set favorites/tags and multi-folder management remain future work.

**Playback:** opening a clip in the desktop app plays it with sound (`clipcraft-player`, `clipcraft-audio-out`) with the full control set in [`playback-controls.md`](playback-controls.md): scrub bar with hover time, ±10 s, frame step, volume and mute, 0.25×–4× speed with pitch kept, loop, audio-track picker, fullscreen with auto-hiding controls, and a keyboard map. Sound is the playback clock, with wall-clock fallback when no device runs. Verified end to end against a file-backed ALSA device (right rate, length, and pitch), but not by ear and not on a real Windows or macOS device; there is no output-device picker, and decoding speed on large 4K or high-bitrate clips is unmeasured on reference hardware.

**Requirements:** define versioned local metadata for library entries, tags/favorites, media path or
fingerprint references, safe atomic index saves, missing-media behavior, and disposable caches.

**Limitations:** ordinary video files are the primary artifacts. Do not create a user-facing
`.ccproj` or persistent editor-draft format; the active trim/cut range can live in memory until
export. Index loss must be recoverable by rescanning folders.

**Acceptance:** restart/rescan reconstructs local clips; round-trip/migration tests pass;
malformed/oversized metadata returns an error; interrupted writes preserve the previous valid index.

## M2 — Windows capture and replay

### M2.1 — Windows display/window capture provider

**Requirements:** capture a selected monitor or application window, report permissions/capabilities,
handle display scaling and hot-plug, and return timestamps/failures through the platform trait.

**Limitations:** Windows 10/11 only in this issue; no injected game hooks, kernel drivers, protected
content bypass, or anti-cheat-sensitive behavior.

**Acceptance:** integration evidence on named Windows versions and GPUs; permission denial,
unsupported protected windows, device loss, and restart produce actionable states without crashing.

### M2.2 — Bounded replay ring and clip hotkey

**Requirements:** store encoded segments within duration/byte quotas, capture the configured
pre-roll, flush optional post-roll, save atomically, and expose armed/recording/buffer status.

**Limitations:** fixed MVP durations and one active capture source; no unbounded raw-frame queue or
permanent recording unless the user enables long recording.

**Acceptance:** deterministic tests prove ring eviction, clip duration, keyframe handling, disk-full
behavior, repeated start/stop, and no loss of already-saved clips.

## M3 — Audio and clip finalization

### M3.1 — Game/system audio and microphone tracks

**Requirements:** capture synchronized audio tracks, select/mute/gain sources, provide a mic test,
and recover when a device disconnects.

**Limitations:** common game/system audio plus one microphone in the first release; advanced per-app
routing, noise suppression, and multi-mic mixing are deferred.

**Acceptance:** generated and real-device tests show bounded A/V drift over a long run, correct mute
and gain, understandable permission errors, and recovery after device loss.

### M3.2 — Clip finalization and local storage limits

**Requirements:** finalize replay segments into local clips, expose progress, enforce a configurable
quota, and clearly distinguish clips/other files/free space.

**Limitations:** no cloud backup or automatic deletion of user clips; quota eviction may remove only
replay-buffer segments or clips the user explicitly selects.

**Acceptance:** full-disk and quota tests preserve existing clips; storage display matches filesystem
measurements within documented rounding.

## M4 — Local library, editor, and export

### M4.1 — Thumbnail library and storage UI

**Status:** partial — local folder browsing, bounded scans, title search, sorting, and file-path
copy are implemented; decoded thumbnails, duration/source metadata, and persistent library metadata
remain open.

**Requirements:** show a dense desktop grid with duration, title, game/source, on-device state, file
size, and date; support import, new edit, filters, rename, tags, favorites, local collections,
delete, reveal-in-folder, and copy-file-path actions. Opening a card shows a local player with a
compact details pane.

**Limitations:** no feeds, profiles, comments, friends, links, cloud buttons, notifications, or
account entry points. Thumbnails use generated or openly licensed media in tests and demos.

**Acceptance:** empty, loading, offline-file, corrupt-file, populated-grid, and clip-detail states are
screenshot-tested; file actions resolve to the correct local path and never call a server.

#### M4.1a — Local folder browsing and clip-first library basics

- **User problem and evidence:** the first library shell required typing a folder path and showed
  every clip with the same generic play tile. That made importing local clips harder to discover
  and left the main screen far from the thumbnail-led library reference.
- **Target behavior:** a Rust native folder dialog sets the local scan path and immediately calls
  the existing `library.scan` command. Cancel keeps the current folder and results. Users can still
  enter a path manually, rescan it, search titles case-insensitively, sort by name or modification
  date, open local file details, and copy a clip path. The library remains the default screen.
- **Affected code and platform:** `apps/clipcraft`, `apps/clipcraft-cli`, and
  `crates/clipcraft-engine` on Windows, macOS, and Linux; it reuses the existing scan command and
  adds a typed local-path command. It does not change media files or the metadata schema.
- **Limitations and fallback:** one non-recursive folder is scanned at a time. Folder-picker
  availability depends on the user's local desktop; manual path entry remains available if the OS
  dialog cannot open. The cards use original vector placeholders because video decoding, actual
  thumbnails, duration, game detection, persistent library metadata, and playback are not built.
  Search filters the current in-memory scan only. No import copy, cloud action, or editor workflow is
  added.
- **Dependencies:** `rfd` provides the native OS folder dialog; the scan still uses the shared
  `library.scan` command. No hosted service or product sidecar is introduced.
- **Acceptance criteria:**
  1. Choosing a readable folder updates the path and displays its video files; the UI calls the
     same scan command used by the CLI.
  2. Canceling the folder dialog leaves the selected path, results, and error state unchanged.
  3. A missing or unreadable folder displays the existing actionable scanner error and retains the
     editable path for recovery.
  4. Search matches title text without case sensitivity; name and newest sorting are stable for
     equal timestamps; empty and no-match states are distinct.
  5. Cards show a clear local video placeholder, title, on-device state, file size, date, and a
     working copy-path action. Selecting a card opens details and does not start the editor.
  6. `cargo xtask ci` passes; the 1440×900 empty library, populated library, and settings screens
     are inspected and captured. No capture, hosted, account, upload, or share-link behavior is
     implied by the UI.
- **Verification:** app unit tests cover scan-on-selection, cancel preservation, case-insensitive
  search, sort ties, and date display; `cargo xtask ci` checks the workspace; manual screenshots
  cover empty and populated states at 1440×900.
- **Estimate:** completed as a focused UI slice; thumbnail decoding remains a separate M1/M4 task.

### M4.2 — Basic cut/trim editor and MP4 export

**Requirements:** support preview, frame-accurate start/end trim, split/cut, undo for the active edit,
and H.264/AAC MP4 export with progress/cancel.

**Limitations:** no multitrack timeline, effects, captions, overlays, plugin system, streaming export,
remote render job, or saved editor project. Preserve originals; export a new normal video file.

**Acceptance:** command-level tests reproduce the UI edit; generated fixtures verify duration,
frame-accurate cut points, audio sync, metadata, cancellation cleanup, and input-file preservation.

## M5 — Reliability and release

### M5.1 — Recovery, installer, and performance gate

**Requirements:** crash recovery, startup/device diagnostics, signed installer, keyboard access,
resource budgets, suspend/resume, two-hour soak, and documented limitations.

**Limitations:** beta claim is Windows-only until other platform rows have equivalent evidence; no
mandatory update service or network telemetry.

**Acceptance:** release checklist includes reproducible capture-to-export evidence with network
disabled, full disk, device loss, suspend/resume, and measured CPU/GPU/memory/power behavior.

## M6 — Additional local platforms

### M6.1 — macOS capture/audio provider

**Requirements:** implement the same local platform trait with ScreenCaptureKit/CoreAudio, permission
guidance, device-loss handling, and the Windows-equivalent verification matrix.

**Limitations:** supported macOS versions and protected-window behavior are stated explicitly; no
server fallback.

**Acceptance:** capture, synchronized audio, recovery, library, and export pass locally on named
hardware and OS versions.

### M6.2 — Linux capture/audio provider

**Requirements:** implement PipeWire/portal capture and audio, document desktop/portal differences,
and reuse the same local replay/export workflow.

**Limitations:** support only explicitly tested desktop/session configurations; unsupported portals
return a clear error; no remote capture service.

**Acceptance:** end-to-end proof names distro, desktop, portal versions, permission flows, audio
devices, and recovery cases.
