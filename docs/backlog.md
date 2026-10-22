# ClipCraft issue backlog

These IDs are stable work-item IDs; create a GitHub issue from the matching template before
implementation and add its number here. Completed items remain as history and evidence pointers.
Every item inherits the constraints in [`issue-requirements.md`](issue-requirements.md). Update the
item status in the same pull request that changes its implementation.

## M0 — product and architecture

### M0.1 — Capture capability and product constraints

**Status:** documented, not measured — the platform matrix is in
[`ADR 0002`](adr/0002-capture-platform-matrix.md) and the remaining decisions are in
[`ADR 0003`](adr/0003-m0-open-decisions.md). Every capture row comes from public documentation;
reference-hardware measurements are the open evidence and belong to the M2/M3 spikes.

**Requirements:** document Windows/macOS/Linux capture and audio APIs, permissions, min OS versions,
encoder availability, and known unsupported sources; record the pure-Rust and offline-core
constraints in an ADR; make `docs/gaps.md` reflect measured evidence only.

**Limitations:** no anti-cheat-sensitive injection, kernel driver, DRM bypass, livestream, account,
upload, web app, or remote-control endpoint.

**Acceptance:** the platform matrix names one supported Windows path and clear failure/fallback
behavior; core workflows work without network/server access; optional backup is not assumed and its
future design is linked; open decisions and their evidence are linked.

### M0.2 — Rust workspace, layering, and quality gate

**Status:** implemented; `cargo xtask ci` passes locally and CI calls the same gate on Linux and Windows runners (the Windows job is unproven until it first runs).

**Requirements:** create a Cargo workspace, `xtask`, layer checker, strict lints, license audit,
asset attribution check, and `cargo xtask ci`; make CI call the same gate; validate all crate
manifests from a clean checkout.

**Limitations:** first-party application and required tool code is Rust. No JS/TS, Node, C/C++
product component, Electron, Tauri, backend, HTTP service, or mandatory external binary runtime.

**Acceptance:** `cargo xtask ci` passes on the supported CI runner; a layer-violation fixture fails;
Rust-only and offline-core/no-required-server checks are documented and enforced where automatable.

### M0.3 — Engine command registry and local CLI skeleton

**Status:** implemented — the shared typed command engine covers library scan/path/probe/thumbnail,
recording settings, capture targets and recording, clip trim, and diagnostics. The CLI and desktop
dispatch through the same engine; commands are local and do not start a server.

**Requirements:** define stable command IDs, typed parameters, validation, error types, and JSON
stdin/stdout behavior; UI and CLI dispatch through the same registry.

**Limitations:** CLI runs locally and does not start a socket, HTTP server, or background daemon. No
current command uploads, syncs, or transmits media. A future MCP adapter is planned over local stdio;
optional encrypted backup is a separate roadmap issue.

**Acceptance:** command list/help and each command's valid, missing, and unknown parameters have
tests; file-changing commands preserve inputs and report failure; test runs need no network.

## M1 — local media and library foundation

### M1.1 — Exact time and bounded media primitives

**Status:** implemented for the current scope — integer ticks, rational frame rates, audio sample
positions, media frame/audio buffers, bounded queues (item and byte caps, drop-oldest or reject
policy), and generated A/V synchronization fixtures are covered by deterministic tests. Buffer
overflow, invalid limits, drift, and trim boundaries are exercised.

**Requirements:** implement integer ticks/rational rates, audio sample positions, checked conversion,
bounded frame/audio buffers, and deterministic generated fixtures.

**Limitations:** no floating-point edit-boundary math; no allocation sized directly from untrusted
input; no FFmpeg or C codec runtime.

**Acceptance:** property tests cover common integer/fractional frame rates, audio-rate conversion,
overflow, zero/invalid rates, and boundary trims; generated audio/video share a monotonic clock.

### M1.2 — Rebuildable local library index

**Status:** implemented with limits — MP4/MOV/MKV/WebM metadata is probed on a background thread
and cached in a versioned per-user JSON index with bounded fields, atomic saves, rescan recovery,
offline entries, and preserved metadata. The desktop restores its last folder and shows cached
thumbnails, favorites, collections, and game groups. Game labels can be guessed from recorder-style
filenames/folders, retained from game-managed saves, or set manually. Scanning is recursive and
cancellable in the desktop, with one active root. Multi-root browsing remains future work. Tags are retained in
the index schema but do not have a complete user-facing editing workflow.

**Containers and export:** MKV and WebM are probed as well as MP4/MOV (`clipcraft-media`). A clip can be trimmed and saved as a new MP4 without re-encoding (`clipcraft-export`, command `clip.trim`, [ADR 0010](adr/0010-trim-export.md)).

**Playback:** opening a clip in the desktop app plays it with sound (`clipcraft-player`, `clipcraft-audio-out`) with the full control set in [`playback-controls.md`](playback-controls.md): scrub bar with hover time, ±10 s, frame step, volume and mute, 0.25×–4× speed with pitch kept, loop, audio-track picker, fullscreen with auto-hiding controls, and a keyboard map. Sound is the playback clock, with wall-clock fallback when no device runs. Verified end to end against a file-backed ALSA device (right rate, length, and pitch), but not by ear and not on a real Windows or macOS device; there is no output-device picker, and decoding speed on large 4K or high-bitrate clips is unmeasured on reference hardware.

**Requirements:** define versioned local metadata for library entries, tags/favorites, media path or
fingerprint references, safe atomic index saves, missing-media behavior, and disposable caches.

**Limitations:** ordinary video files are the primary artifacts. Do not create a user-facing
`.ccproj` or persistent editor-draft format; the active trim/cut range can live in memory until
export. Index loss must be recoverable by rescanning folders.

**Acceptance:** restart/rescan reconstructs local clips; round-trip/migration tests pass;
malformed/oversized metadata returns an error; interrupted writes preserve the previous valid index.

## M2 — Windows capture and replay

### M2.P1 — Replay and pixel-conversion performance

Local performance work item: reduce save-time payload copying, repeated GOP scans and redundant
pixel writes without weakening clip safety or bounds. Scope, limitations, acceptance targets,
machine/workload details, before/after measurements and follow-up work are recorded in
[`performance.md`](performance.md). Dependencies: existing replay/recording pipeline; no new crates.
Implemented and measured; `cargo xtask ci` passes on Windows with serial tests and the CI-pinned
audit tool. The default parallel recovery test remains load-sensitive. Real-hardware capture,
game-frame-time and release-soak checks remain open; see the review for details.

### M2.1 — Windows display/window capture provider

**Status:** implemented to compile; runtime evidence open — Windows Graphics Capture and global
hotkey adapters compile for `x86_64-pc-windows-gnu`; they have not run on a Windows machine.

**Requirements:** capture a selected monitor or application window, report permissions/capabilities,
handle display scaling and hot-plug, and return timestamps/failures through the platform trait.

**Limitations:** Windows 10/11 only in this issue; no injected game hooks, kernel drivers, protected
content bypass, or anti-cheat-sensitive behavior.

**Acceptance:** integration evidence on named Windows versions and GPUs; permission denial,
unsupported protected windows, device loss, and restart produce actionable states without crashing.

### M2.2 — Bounded replay ring and clip hotkey

**Status:** implemented and end-to-end verified on Linux/X11 with synthetic capture — bounded
pre-roll, post-roll, global save hotkey, MP4/MKV writing, framing, storage limits, and failure
cleanup have automated/virtual-display evidence. Windows capture runtime and real-game performance
are open; see [`gaps.md`](gaps.md).

**Requirements:** store encoded segments within duration/byte quotas, capture the configured
pre-roll, flush optional post-roll, save atomically, and expose armed/recording/buffer status.

**Limitations:** fixed MVP durations and one active capture source; no unbounded raw-frame queue or
permanent recording unless the user enables long recording.

**Acceptance:** deterministic tests prove ring eviction, clip duration, keyframe handling, disk-full
behavior, repeated start/stop, and no loss of already-saved clips.

## M3 — Audio and clip finalization

### M3.1 — Game/system audio and microphone tracks

**Status:** implemented and verified with generated sources (see `docs/gaps.md` and ADR 0009); real-device runs are open.

**Requirements:** capture synchronized audio tracks, select/mute/gain sources, provide a mic test,
and recover when a device disconnects.

**Limitations:** common game/system audio plus one microphone in the first release; advanced per-app
routing, noise suppression, and multi-mic mixing are deferred.

**Acceptance:** generated and real-device tests show bounded A/V drift over a long run, correct mute
and gain, understandable permission errors, and recovery after device loss.

### M3.2 — Clip finalization and local storage limits

**Status:** implemented and tested — MP4/MKV recording, progress/cancel, local storage limit,
crash recovery, and full-disk preservation are verified with generated sources and Linux tests.
Windows filesystem and runtime behavior remain open.

**Requirements:** finalize replay segments into local clips, expose progress, enforce a configurable
quota, and clearly distinguish clips/other files/free space.

**Limitations:** no cloud backup or automatic deletion of user clips; quota eviction may remove only
replay-buffer segments or clips the user explicitly selects.

**Acceptance:** full-disk and quota tests preserve existing clips; storage display matches filesystem
measurements within documented rounding.

## M4 — Local library, editor, and export

### M4.1 — Thumbnail library and storage UI

**Status:** implemented with limits — local folder browsing, bounded recursive scans, search,
sorting, decoded/cached thumbnails, probed media metadata, persistent index, favorites, collections,
game labels, missing-file relink, playback, and local file actions are available. Multi-folder
browsing and a full free-form tagging workflow are open; game-managed saves now retain game labels.

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
  dialog cannot open. This historical slice used original vector placeholders; current thumbnails,
  media details, persistent metadata, and playback are documented in M1.2 and M4.1.
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
- **Status:** completed. Its original acceptance evidence describes the first library shell; current
  behavior and additional limits are recorded in M1.2, M4.1, and [`gaps.md`](gaps.md).

#### M4.1b — Recursive scans and captured-game metadata

**User outcome / evidence:** the existing library misses clips in game subfolders, and recorded
clips lose the game identity already known by automatic arming. Complete this workflow in the
shared Rust engine, desktop and CLI, without new dependencies or media sidecars.

**Requirements:** traverse one selected root, skip symlinks, limit the entire traversal to 20,000
entries, 5,000 recognized videos and eight child-folder levels. Run traversal on one desktop worker
with one replaceable pending request. Cancel/replacement must discard stale results; only the UI
thread merges scans into the current index. Preserve favorites, collections, probes, manual game
labels and offline entries. Snapshot the active game when Save is pressed and register it after a
successful save. A manual game overrides captured metadata, then folder and filename guesses.

**Limitations:** one root is browsed at a time; hard-linked files at distinct paths remain separate
clips. Non-UTF-8/oversized paths, unreadable children and exceeded limits fail the whole scan with
an actionable error instead of publishing incomplete results. Cancellation cannot interrupt an OS
filesystem call already blocked on a disconnected drive. Captured-game metadata belongs to the
local rebuildable index; deleting it loses those labels, as with manual metadata. No new game
detection or Windows hardware support is claimed. Missing/invalid game labels must never prevent
saving the video; index failures keep the media and report a label-persistence warning.

**Acceptance / verification:** generated nested folders appear once, folder game filters/counts
update, symlink loops are skipped, all global limits are enforced, cancellation/replacement and
errors preserve the prior view/index, and metadata edits made while scanning survive completion.
Generated recording tests verify the game snapshot, restart/rescan persistence, manual override
and restoration of the recorded label when the override is cleared. Check hostile metadata and
permissions, inspect native before/after screenshots and keyboard access to Cancel, then run
`cargo xtask ci` and the documentation gate. No Windows machine is required for this slice.

**Documentation:** README, gaps, roadmap, architecture, testing and CLI help ship with this change.
**Dependencies / estimate:** existing M4.1 and M7.1 engine paths; one focused implementation.
**Status:** implemented; generated Linux tests and native UI evidence are recorded in `testing.md`.

#### M4.1c — Advanced controls within their categories

**Problem / evidence:** the user found the custom-game form cramped and asked to move it behind
General's advanced switch. The old switch preceded Startup/Help and added a separate Advanced
category containing unrelated capture, encoder and audio controls.

**Scope / requirements:** Rust desktop settings only. Make the switch the final General setting;
keep navigation fixed. Append a divider and **Advanced settings** heading before each relevant
category's extra controls: custom games in Games, aspect ratio in Capture, encoder effort/keyframes
in Quality, sample rate/channels in Audio. Use padded, responsive fields directly on the page,
without a card/background, for adding and editing custom rules; preserve drafts, existing matching modes and the shared settings save command.

**Limits:** disclosure only; hiding controls must preserve their values and active custom rules.
No new capture capabilities, dependencies, server or process refresh on reveal. Retain the 64-rule
and bounded-input limits. No empty advanced section for categories with no extra controls. A failed
or empty app refresh keeps manual entry available; malformed IDs retain an editable draft.

**Acceptance / verification:** Tab/Enter toggles General's bottom switch and adds a custom game;
all four categories expose the correct controls under the heading with no new nav item. Turning
advanced settings off preserves a custom rule's detection and other values. Picking a non-Steam
app clears a previous Steam ID. Draw narrow/large layouts with existing rules, inspect native
1440×900 before/after screens and a narrow window, and run `cargo xtask ci` plus the docs gate.

**Docs:** README, gaps, roadmap, settings guide, architecture, testing, catalog guide and ADR 0015.
**Dependencies:** M4.1/M7.1a. **Estimate:** one focused UI change. **Status:** implemented; Linux
CI, keyboard/accessibility and native 1440×900/960×900 inspection pass. See testing.md for evidence.

### M4.2 — Basic cut/trim editor and MP4 export

**Status:** partial, core trim delivered — a clip can be previewed, trimmed, and exported as a new
MP4 without re-encoding, with progress/cancel and input preservation. The editor remains intentionally
basic; re-encoding, arbitrary split workflows, and quality/size presets are not implemented.

**Requirements:** support preview, frame-accurate start/end trim, split/cut, undo for the active edit,
and H.264/AAC MP4 export with progress/cancel.

**Limitations:** no multitrack timeline, effects, captions, overlays, plugin system, streaming export,
remote render job, or saved editor project. Preserve originals; export a new normal video file.

**Acceptance:** command-level tests reproduce the UI edit; generated fixtures verify duration,
frame-accurate cut points, audio sync, metadata, cancellation cleanup, and input-file preservation.

### M4.4 — GIF and loudness evening

**Status:** implemented and tested with generated clips on Linux; no Windows/macOS or real-game
evidence. Engine commands `clip.export_gif` and `clip.normalize`, the CLI
(`clip gif|normalize`) and the trim panel's output menu share one implementation. See [ADR 0018](adr/0018-gif-and-loudness-export.md).

**Requirements:** save a short range as a looping GIF; bring a range's sound to a common loudness without clipping.
Preserve originals; never overwrite; cancel leaves nothing behind; work offline.

**Limitations:** GIF is at most 30 s, 960 px wide, 20 fps and has no sound. Joining clips is not
included; it belongs to the planned editor with transitions. Loudness evening needs AAC sound, applies one gain to every track, holds the gain back at
-1 dBTP, and re-encodes the sound (the picture is copied).

**Acceptance:** generated fixtures verify GIF size, frame count, delay and loop; normalised output measures at the target within 1 LU, keeps a
click within 50 ms of its place, never exceeds full scale, and decodes cleanly. Bad inputs,
cancellation and collisions leave no partial file and do not touch the sources.

**Open:** real-game and Windows evidence; speed on representative PCs (see
[testing.md](testing.md)); a clip editor with joins and transitions; WebM/AV1 output.

### M4.3 — Local export size and quality presets

**Status:** initial software implementation built — `clip.export` and the player trim panel offer
estimated 10 MB, 25 MB, and custom 1–500 MB H.264 MP4 exports. Original-quality trim remains the
default. Generated MP4/MKV tests verify decoding, trim/audio alignment, actual-size reporting,
cancellation, malformed inputs, and collision cleanup. See [ADR 0013](adr/0013-local-compressed-export.md).
Broader quality/codec controls and representative 30/60-second performance evidence remain open.

**Requirements:** add an offline export job that can re-encode a selected trim to a small set of
clear quality/size presets. Include an estimated Discord-friendly target, a custom size/quality
choice if the encoder supports it, visible progress and cancellation, and an output-size report.
Keep the original clip unchanged and write a normal local video file.

**Limitations:** no upload, link creation, Discord integration, login, server, or guarantee that a
variable-bitrate file meets a third-party platform's changing size limit. Do not ship a runtime
FFmpeg dependency. Any encoder dependency must pass license, maintenance, Rust-safety, and performance
review before adoption.

The implemented slice accepts H.264/AAC MP4/MOV/MKV, copies up to 16 AAC tracks, and re-encodes video
in software at up to 720p/30 fps without upscaling. It is bounded to 30 minutes per job. HDR and
non-square pixels are refused. The destination filesystem must support hard links. Audio bitrate
reduction, hardware export, custom dimensions/FPS, and additional codecs are outside this slice;
the original clip remains intact on success or failure.

**Acceptance:** generated fixtures verify output decodes, selected trim range and A/V sync are
preserved, output size/quality controls behave within documented tolerance, cancel/error leaves no
partial file, input bytes remain unchanged, and the feature works with network access disabled.

**Verification:** deterministic encoder tests plus ffprobe/ffmpeg as external test oracles; measure
encode time and output size for representative 30 s and 60 s clips at each preset.

**Dependencies:** the existing reviewed FilmCraft Rust decoder, software encoder, conversion and
muxer, plus the workspace image resizer. No new upstream revision or license exception. The
software-only implementation does not require GPU evidence; representative-machine throughput
still needs measurement. Estimate: initial implementation complete, remaining controls TBD.

## M5 — Reliability and release

### M5.1 — Recovery, installer, and performance gate

**Status:** partial — crash and full-disk recovery, Linux package, diagnostics, keyboard access, and
resource soaks are implemented, including a passed two-hour Windows/AMD generated-window 1080p30
run. Signed Windows/macOS installers, 1080p60, real screen reader, real-device audio, and broader
reference-PC performance evidence remain open; see
[`release-checklist.md`](release-checklist.md).

**Requirements:** crash recovery, startup/device diagnostics, signed installer, keyboard access,
resource budgets, suspend/resume, two-hour soak, and documented limitations.

**Limitations:** beta claim is Windows-only until other platform rows have equivalent evidence; no
mandatory update service or network telemetry.

**Acceptance:** release checklist includes reproducible capture-to-export evidence with network
disabled, full disk, device loss, suspend/resume, and measured CPU/GPU/memory/power behavior.

### M5.2 — Low-copy GPU capture-to-encode path

**Status:** Windows SDR WGC-to-D3D11-to-Media-Foundation is implemented with bounded owned surface
leases and CPU fallback. Generated-window AMD runs confirm zero CPU readbacks at matching dimensions;
both native and CPU paths still encode about 32 fps when requesting 1080p60. See
[the measurements](windows-performance.md). HDR/scaling, other GPUs, real-game frame times,
GPU/power comparisons and device-loss integration remain open. The two-hour Windows/AMD generated-window
1080p30 soak passed with eight drops, zero restarts and 24 decoded clips; 1080p60 remains open.
M5.2 is partial.

**User problem:** full-frame transfers to CPU memory and CPU-side conversion can use processor time
and GPU bandwidth while the user is playing. Reduce that work for supported capture and encoder
pairs without changing the offline replay/save workflow.

**Target behavior:** where the OS capture API and the selected hardware encoder can share compatible
GPU resources, keep each picture on the same GPU through capture, scaling/color conversion, and
encoding. Place encoded packets in the existing bounded replay buffer. Show the active capture and
encoder path in diagnostics; keep the current CPU-frame path as a clear fallback when resources,
drivers, or permissions do not support the low-copy path.

**Requirements:** first add a platform feasibility spike and surface-lifetime design. Evaluate
PipeWire DMA-BUF import and per-user Linux capture paths; evaluate D3D11 texture handoff from Windows
Graphics Capture to supported hardware encoders; consider ScreenCaptureKit/IOSurface and
VideoToolbox after macOS capture exists. Keep OS and GPU handles inside `clipcraft-platform`, use
bounded frame/surface queues, report device loss and fallback reasons, and preserve frame timing,
HDR/color handling, audio sync, and crash-safe saving. Measure CPU use, game frame-time impact, GPU
engine use, dropped frames, memory, and power on named reference systems at 1080p60.

**Limitations and fallback:** this is an optimization, not a prerequisite for local capture. Do not
load a driver, inject into a game, require a privileged KMS helper, or silently switch capture APIs.
Keep portal consent and compositor permission behavior intact. Do not ship GPU Screen Recorder as a
runtime: its GPL-3.0-only license and non-Rust/FFmpeg stack conflict with ClipCraft's
permissive-dependency and Rust-runtime rules. Use it as a public behavior reference only. Do not claim
support for every compositor or promise sub-1% CPU until each claimed combination is measured;
monitor capture through KMS may need elevated access, while portal delivery and drivers vary by
compositor and GPU.

Implement the capability independently in Rust from public operating-system and vendor API
documentation. Do not translate or copy GSR source: that would make this a source port with GPL
licensing implications, outside the current permissive-dependency policy. Keep the existing CPU
capture path available throughout development, and enable a native path only for combinations that
pass capture, audio-sync, device-loss, and performance checks.

**Affected code/platforms:** `clipcraft-capture`, `clipcraft-record`, and `clipcraft-platform`;
NVIDIA, AMD, and Intel hardware only where the native capture and encode APIs interoperate. Initial
work targets Linux and Windows; macOS follows its capture provider.

**Acceptance:** on each claimed platform/GPU path, a real-device capture-to-file run avoids a
full-frame CPU readback and re-upload in ClipCraft's pipeline, preserves pixel correctness and
timestamps, stays within the configured surface/queue bounds, and recovers or reports a clear error
on device loss. The existing CPU fallback remains selectable and produces equivalent clips. Publish
CPU/GPU use, game frame-time delta, dropped frames, resolution/FPS, driver, compositor, and power
measurements. Sub-1% CPU is a stretch target for named systems, not a cross-platform guarantee.

**Verification:** fake surface-pool tests prove bounded ownership and cleanup; integration tests
compare generated colors, timestamps, and A/V sync between GPU and CPU paths; real hardware tests
cover device loss, permission denial, compositor changes, and repeated start/stop. Do not mark a
vendor, compositor, or OS combination supported from compilation alone.

**Dependencies:** M2/M3 capture and recording APIs, M5.1 performance baseline, and the relevant
platform provider (M6.2 for Linux Wayland, future macOS capture for Apple). **Estimate:** large; split
the work by platform after the feasibility report.

## M6 — Additional local platforms

### M6.1 — macOS capture/audio provider

**Status:** not started.

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

**Status:** X11 capture, VA-API, and PulseAudio/PipeWire sound are done; Wayland (portal + PipeWire)
is built and partly verified, see [docs/wayland.md](wayland.md). Open: a run on GNOME, KDE and a GPU
wlroots desktop, restoring the chooser's choice, and DMA-BUF buffers.

## M7 — Game-aware capture and reference review

### M7.0 — Compare reference projects and record product decisions

**Status:** partial — Vice and ClipRecorder source/docs and repository metadata were inspected on
2026-10-21; two small Rust source ports are now implemented under permission reported by the
ClipCraft maintainer. Review of Segra, ScreenCraft, Medal, and ShadowPlay remains open.

**User problem and evidence:** suggestions include Vice, ClipRecorder, Segra, ScreenCraft, Medal, and
NVIDIA ShadowPlay. Their capture lifecycle, game awareness, compression, controller, editor, and
community choices may inform ClipCraft. This initial review checked Vice at
[`7e4b2a3`](https://github.com/splitshotz/Vice/commit/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c)
and ClipRecorder at
[`153025b`](https://github.com/By183/ClipRecorder/commit/153025b4c88ed42a23a97e030ce140c2fc54949d)
on 2026-10-21.

**Initial findings — Vice:** its [README](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/README.md)
describes a Linux game clipper with game labels/playlists,
Discord Rich Presence, configurable hotkeys, longer sessions with markers, a keyboard overlay, and
a built-in multitrack editor. Its game identification is based on focused-window/compositor
adapters; it does not implement the requested Game-only start/stop lifecycle. Its documented
installer can enable a user daemon at login, and its default runtime is always-on even when no game
is focused (Nix exposes an `autoStart` opt-out). The longest-match game-name rule in
`vice/main.py` has been ported to Rust as a matching primitive; process observation and automatic
arming remain separate work. It uses an external `gpu-screen-recorder` backend,
a Python package, and a Qt WebEngine/TypeScript UI. Its package metadata declares
GPL-3.0-or-later ([`pyproject.toml`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/pyproject.toml)). The broader source, assets, and
runtime are outside ClipCraft's port scope; the selected matching function is included under
reported permission. The README's low-CPU claim
does not establish idle GPU use. The user reported high idle GPU use while testing Vice; that report
is anecdotal and unverified. ClipCraft must measure both CPU and GPU while no game is running.

**Initial findings — ClipRecorder:** its [README](https://github.com/By183/ClipRecorder/blob/153025b4c88ed42a23a97e030ce140c2fc54949d/README.md)
and [`recorder.cpp`](https://github.com/By183/ClipRecorder/blob/153025b4c88ed42a23a97e030ce140c2fc54949d/src/recorder.cpp)
describe Windows Graphics Capture
frames converted by a D3D11 video processor to NV12, a generic Media Foundation H.264 hardware MFT,
an encoded time ring, WASAPI loopback/microphone audio, and MP4 muxing when the user saves. Its
timestamp gap, overlap, and microphone recovery policy from `src/audio_timeline.h` has been ported
to a Rust utility. This
closely overlaps ClipCraft's Rust WGC provider, Media Foundation encoder adapter, recorder ring, and
local export path, so it does not currently offer a missing subsystem to transplant. It starts
recording when the app launches, which does not fit Game-only default behavior. The repository is
C++20/Windows-only and has no license grant: GitHub reports no license and the README states
“unlicensed — all rights reserved.” The port is included under permission reported by the ClipCraft
maintainer; see [`upstream-ports.md`](upstream-ports.md). Do not add its C++ app, assets, or runtime.
Public Windows API documentation and independent test scenarios remain useful references.

**Remaining requirements:** review public documentation and repositories for Segra, ScreenCraft,
Medal, and ShadowPlay. Complete the comparison matrix for game detection/idle behavior,
capture/encoding path, audio, replay controls, editor, export, controller support, backups, and
external services. For any future code candidate, check license, maintenance, Rust fit, `unsafe`, and
dependency direction before proposing reuse. Link primary sources and date the findings.

**Reuse decisions for the two inspected projects:**

| Behavior or technique | Decision |
| --- | --- |
| Vice game matching and game labels | Port the longest-match rule into `clipcraft-engine`; add a user-editable allowlist and low-overhead process lifecycle watcher as M7.1. Do not bundle Vice's game data. |
| Vice keyboard overlay and Rich Presence | Keep as later optional features in M10; default off, clearly scoped permissions, no clip transmission. |
| Vice multi-track editor and session markers | Defer. The first editor expansion remains a one-track evaluation after capture/platform work. |
| Vice public links, tunnel, social sharing, and always-on daemon | Reject for ClipCraft's local-first/Game-only direction. |
| ClipRecorder WGC/D3D11/MFT/ring-buffer pipeline | Port only its timestamped audio gap/overlap/microphone-recovery policy as a Rust utility. ClipCraft already has Rust WGC, MF encoder, replay, and export components; validate these on Windows hardware rather than duplicate them. |
| ClipRecorder launch-time capture | Reject as the default. The replay ring should remain stopped until an allowlisted game is running. |

**Limitations:** the two source ports are limited to the documented algorithms and depend on the
maintainer-reported permission in [`upstream-ports.md`](upstream-ports.md); do not copy unrelated
source, bundled game data, artwork, game footage, protocols, or private APIs. Do not add
`gpu-screen-recorder` or another non-Rust recorder as a product runtime. Segra/community and hosted
sharing features are research context only and remain out of scope.

**Acceptance:** the matrix separates verified facts from assumptions; each borrowed behavior has a
clear product decision (`adopt`, `adapt`, `defer`, or `reject`); any dependency candidate has a
license/API/safety note or is explicitly not recommended. The two reviewed repositories have
documented product decisions, and each completed algorithm port has parity tests and keeps its
upstream source references in sync with future changes.

**Verification:** maintainers review source links and decisions in `PLAN.md` and
[`upstream-ports.md`](upstream-ports.md); focused Rust tests verify the ported algorithms. Broader
repository review remains open.

**Dependencies:** none. **Estimate:** small research task.

### M7.1 — Game-only automatic replay arming

**Status:** partial; the user-managed allowlist, process matching, Game-only default, manual pause,
and X11/Windows process-to-window adapters are implemented. Real-game and Windows runtime evidence,
and Wayland matching remain open. Library auto-tagging from game-managed saves is implemented (M4.1b).

**User problem and target behavior:** ClipCraft should record only when a game is started, so the app
must not continuously capture an idle desktop. **Game-only** is the default. Starting a recognized
game arms the bounded replay buffer; closing its process stops capture and releases the
buffer. The user can pause capture until all matching games exit, or disable Game-only mode for
manual capture. Start-on-launch is ignored in Game-only mode.

**Requirements:** provide an editable per-game allowlist and a platform process-observation adapter.
Match only process metadata (executable basename and optional exact Steam app ID); do not read
process memory, inject code, install drivers, or infer protected game state. Show which game caused
capture to arm, provide an immediate stop/pause control, and never auto-start at login or capture
while no selected game is running. Add fake process-event tests and real platform evidence for
every supported OS.

**Limitations and fallback:** 348 built-in games are recognized automatically (M7.1a);
the pinned Vice catalog is bundled as Rust data. Users can use custom rules from the running-process picker or enter
executable names and Steam app IDs. Steam IDs are read only on Linux. Process names
can be ambiguous, elevated or sandboxed games may be hidden, and anti-cheat/protected games can deny
capture. Windows mapping is compile-only here; Wayland does not expose game-window process IDs, and
macOS capture is not implemented. On unsupported or permission failures show a clear message and
remain idle; never fall back to capturing the desktop silently. Android game launch cannot bypass
MediaProjection consent or background-service restrictions.

**Affected code/platforms:** `apps/clipcraft`, `crates/clipcraft-engine`, and
`crates/clipcraft-platform`; Windows/Linux/macOS desktop providers. Android remains outside this
implementation item.

**Acceptance:** with no recognized process, Game-only mode leaves capture and the replay ring
stopped; starting an allowlisted process arms once; duplicate/matching helper processes do not create
duplicate sessions; process exit stops capture without corrupting already saved clips; failures and
permission denial leave capture stopped; the UI identifies the active game and manual stop pauses
automatic restart until all matched processes exit. The user can disable Game-only mode to use
manual capture.

**Verification:** engine tests cover process matching, exact IDs, duplicate observations, exit,
restart, and manual pause; an X11 integration test maps a fake process ID to a window. The app's
Linux build and real-game flow still need capture-hardware verification; Windows needs a real-device
run. Measure idle CPU/GPU use before treating the resource budget as proven.

**Dependencies:** M6 platform providers and M7.0 research. **Estimate:** TBD after platform API
spikes.

### M7.1a — Automatic known-game recognition

**Problem / evidence:** requiring preset imports adds setup for already-known games. Vice loads its
bundled list automatically and checks custom rules first (`_match_game`, revision `7e4b2a3`). The
user requested this behavior, then requested the complete Vice list. Bundle all 347 Vice titles
as Rust data plus the existing Bedrock entry, preserving existing IDs/variants; record pinned
source, adaptations and maintainer-reported permission.

**Requirements:** recognize built-ins by default, check custom rules first, support a global switch
and per-built-in exclusions, and preserve these choices across reloads. Use exact built-in
executable/Steam identifiers; custom rules retain their existing matching mode. The UI presents a
searchable known-game list plus custom overrides. UI/CLI share `recording.set_builtin_games` for
atomic exclusion changes. Keep game-only start/stop, manual capture and pause behavior.

**Limits:** identification fixtures do not establish actual per-game/capture support. Steam process
IDs from process environments are Linux-only; M7.1b adds local Steam manifests and installation-path
mapping. Generic Java runtimes still cannot identify an arbitrary game by basename alone. No artwork, runtime database downloads, network service or launcher runtime is added.
Omit seven launcher/generic aliases; retain their titles and other identifiers.
Missing new preference fields default automatic recognition on, preserving the old game-only/manual
mode and custom rules; explicit off/exclusions remain off. See ADR 0015. Custom rules are capped at
64; built-in exclusions/batch requests are capped at 1,024 and normalized to known IDs. No idle-performance claim is made.

**Acceptance / verification:** default settings recognize known executable/Steam fixtures without
custom setup; explicit custom names win even over built-in IDs; global-off/per-game exclusions
work and persist; unknown/oversized updates have no partial effect; every Vice title is present. Test all-348
exclusion persistence within the existing settings-file budget, old-settings migration,
malformed exclusions, matching lookalikes, arming start/stop and no-rule idle. Inspect native UI,
keyboard filtering/exclusion controls, isolated CLI and `cargo xtask ci`. Update README, roadmap,
gaps, architecture, testing and catalog provenance together. **Dependencies:** M7.1.
**Estimate:** one focused change. **Status:** implemented; live-game/Windows evidence remains open.

### M7.1b — Background discovery, local Steam games and desktop continuation

**User outcome / evidence:** before this issue, process matching ran on the interface thread; games
beyond the bundled list needed custom setup. The user requested the proposed background/local-Steam work
and an opt-in desktop continuation when a running game closes or crashes.

**Scope / requirements:** one persistent bounded worker for process discovery, matching and local
Steam manifests; obsolete generations cannot change capture. UI/CLI share an offline Steam scan
command. Preserve custom priority and exclusions. Games-only stays the default. Add a timed desktop mode (15–3,600 seconds after a successfully
captured game exits) and an explicitly chosen constant desktop/game mode. Re-entry cancels the
timer and switches to the game; its next exit gets a fresh timer. Preserve final replay on automatic
source changes/expiry; use fresh buffers and no old game label on desktop clips. Stop pauses both
desktop modes until Start. Show the active source/countdown and keep source/capture errors safe.

**Limits:** bounded VDF parsing, library/manifest/process counts and one pending worker request;
no network, code injection, game-file inspection or added runtime. Unknown roots, unavailable
libraries, malformed metadata and scanner failures report limitations; no false game-exit result.
Keep capture/window permissions and existing storage limits. Windows/live-game evidence remains
unavailable here; use synthetic process/source fixtures. See ADR 0016 for privacy/defaults.

**Acceptance / verification:** check alternate Steam libraries, corrupted/oversized metadata,
source precedence/exclusions, stale worker results, responsiveness while blocked, worker failure,
launch/arm/save/exit/crash/default-off/fallback/return/pause/disable flows and saved-game labels.
Inspect UI at 1440×900, exercise keyboard toggles, and run `cargo xtask ci` plus documentation gate.
Update README, gaps, roadmap, architecture, testing, settings/catalog guides and ADR/provenance.
**Dependencies:** M7.1/M7.1a, M4.1c. **Estimate:** multi-part change. **Status:** implemented and verified on Linux with generated sources, native X11 switching,
keyboard/screenshots, valid saved media and the full gate; Windows/live-game validation remains open.
See [testing.md](testing.md#m71b--background-discovery-and-desktopgame-switching-2026-10-22).

## M8 — Desktop platform parity and Android feasibility

### M8.1 — Cross-platform desktop support matrix

**Status:** planned; M6 Linux work is partial and macOS is not started.

**Requirements:** close the Windows/Linux/macOS matrix for game-only arming, capture, audio, hotkeys,
replay save, playback, trim/export, permissions, recovery, and installers using the shared Rust core
and narrow platform adapters. Publish explicit supported OS versions and known unsupported capture
sources.

**Affected code/platforms:** `crates/clipcraft-platform`, media/audio adapters, engine, desktop app,
and release packaging on Windows, Linux, and macOS.

**Limitations:** do not claim support from compilation alone. DRM/protected content and
anti-cheat-sensitive injection remain unsupported; no platform may silently use a hosted capture
fallback.

**Acceptance:** each advertised OS passes named-device end-to-end capture-to-export, game-start/exit
lifecycle, permission denial, device loss, and restart tests. Differences and workarounds appear in
the README and release checklist.

**Verification:** platform CI plus real-device evidence, named OS/API versions, GPU, audio devices,
and measured performance. **Dependencies:** M6, M7.1. **Estimate:** TBD per platform.

### M8.2 — Android capture feasibility study

**Status:** planned research spike; Android is not a supported platform commitment.

**Requirements:** test whether a Rust-first client can use Android's user-visible screen-capture
permission, foreground service, storage, audio, controller, and codec APIs while staying within
battery and thermal budgets. Deliver a platform/dependency matrix, a small capture prototype only if
permitted by platform policy, and a go/no-go recommendation.

**Affected code/platforms:** Rust platform/UI compatibility and Android APIs; this is a research
spike, not a shipping-platform implementation.

**Limitations:** never capture before explicit OS consent or hide a foreground recording indicator.
Game launch cannot grant screen-capture permission automatically. No Android feature is promised by
the desktop roadmap.

**Acceptance:** the report covers user consent and revocation, background restrictions, process/game
detection, audio permission, suspend/resume, low storage, thermal behavior, and Rust UI/runtime
compatibility; any prototype records only after visible consent and can stop immediately.

**Verification:** named Android API levels and physical devices; record battery/thermal measurements
or mark them uncollected. **Dependencies:** M8.1 shared core boundaries. **Estimate:** small spike.

## M9 — Optional encrypted backup

### M9.1 — User-configured client-side-encrypted backup and restore

**Status:** proposed; no backup provider, encryption format, or key recovery mechanism is selected.

**User problem:** the user wants a way to protect local clips with encrypted cloud backups while
keeping ClipCraft usable offline and avoiding a ClipCraft-hosted server or sharing service.

**Requirements:** first complete a design ADR that selects a provider and reviewed Rust cryptography
and specifies local secret storage, passphrase/recovery key, ciphertext versioning, visible metadata,
chunking, retry, integrity checks, restore, delete, credential revocation, and key-loss behavior. Then
implement opt-in backup of user-selected clips and required library metadata. Encrypt on the device
before transfer. Support cancellation and resume without altering originals; explain that losing the
key makes ciphertext unrecoverable.

**Affected code/platforms:** a proposed Rust backup/provider crate, engine commands, UI/CLI, and
platform secret storage on supported desktop OSes. Exact crate boundaries follow the design ADR.

**Limitations:** no backup is enabled by default; no ClipCraft account, hosted bucket, key escrow,
social sharing, or automatic cross-device library sync. A network/provider outage must not block
capture, local playback, edit, or export. Provider object names and other unavoidable metadata must
be documented.

**Acceptance:** an explicit setup flow names the provider and key recovery warning; no bytes are
sent before confirmation; the provider receives ciphertext only; interrupted backup resumes safely;
integrity failure is surfaced; a fresh installation restores and decrypts synthetic clips with the
recovery material; losing credentials/key returns a clear unrecoverable state; originals are never
modified or deleted as a side effect.

**Verification:** local fake-provider tests exercise retries, cancellation, corruption, deletion, and
restore; crypto uses vetted crates with license/security review, test vectors, and no custom cipher;
a disposable cloud account test verifies the actual provider path with synthetic footage. Audit logs
and diagnostics contain no keys, tokens, or private paths.

**Dependencies:** M7.0 reference review, key-storage/security ADR, and provider selection. **Estimate:**
TBD after design spike.

## M10 — Controller, presence, overlays, and extensions

### M10.1 — Controller shortcut bindings

**Status:** planned.

**Requirements:** allow the user to bind a supported controller button/chord to save the replay,
start/stop a session, or take a screenshot; expose binding and conflict state in settings. Reuse the
same engine commands as keyboard hotkeys and keep controls accessible by keyboard.

**Affected code/platforms:** `clipcraft-platform`, engine hotkey/command registry, and desktop
settings on Windows, Linux, and macOS.

**Limitations:** support only controllers and OS APIs verified in the matrix. Do not emulate a
controller, inject input into games, or require a game launcher. Game focus/anti-cheat behavior may
prevent global input observation.

**Acceptance:** binding persists locally, conflicts are reported, unplug/replug recovers, and a
button press invokes the same tested command as its keyboard equivalent without consuming game
input. **Verification:** fake-device tests plus named controller/OS integration evidence.

**Dependencies:** M8.1. **Estimate:** TBD.

### M10.2 — Optional Discord Rich Presence

**Status:** planned.

**Requirements:** provide opt-in local Rich Presence that can show ClipCraft activity and, only if
the user enables it, the detected game name. Explain the exact fields before enabling; turn off and
clear presence when the user exits or disables it.

**Affected code/platforms:** desktop app and a small Rust presence adapter on Windows/Linux/macOS;
Discord's local IPC availability is optional.

**Limitations:** no clip upload, link, Discord login inside ClipCraft, friends/community surface, or
collection of Discord account data. Discord availability is optional and must not affect capture.

**Acceptance:** default is off; enabling/disabling and Discord unavailable states are clear; stopping
ClipCraft clears the presence; capture remains functional when Discord is missing or blocked.

**Verification:** mock IPC tests and a manual opt-in desktop check; inspect logs for no token, clip
path, or private metadata. **Dependencies:** M7.1 game identity. **Estimate:** TBD.

### M10.3 — Built-in keyboard/mouse overlays and themes

**Status:** planned.

**Requirements:** prototype an optional, user-positionable keyboard/mouse overlay and a small set of
declarative visual themes. Overlays are composited into exported/captured footage only after the user
turns them on and chooses what input categories to reveal. Provide preview and a clear recording
indicator for overlay state.

**Affected code/platforms:** Rust UI/settings, capture/render composition, and export on verified
desktop platforms.

**Limitations:** no keystroke logging outside the active capture session, no passwords/private text,
no hidden capture, and no third-party shader code in this first slice. The overlay must be optional
and must not intercept game input.

**Acceptance:** user can disable overlay globally, choose allowed input categories, preview placement,
and verify export contains only selected events; accessibility and performance budgets are met.

**Verification:** synthetic input tests, screenshot/video fixture comparison, privacy tests for
excluded input, and measured capture overhead. **Dependencies:** M8.1, editor/compositor capability
spike. **Estimate:** TBD.

### M10.4 — Plugin and shader extension design spike

**Status:** planned research; no plugin runtime is approved.

**Requirements:** compare declarative packages, Rust/WASM plugins, and other Rust-compatible
isolation options for themes, small functions, and shaders. Define versioning, capability grants,
filesystem/network access, CPU/GPU and memory budgets, crash isolation, signing/trust, and removal.

**Affected code/platforms:** a future Rust engine extension boundary and desktop app; runtime and OS
matrix remain undecided pending this spike.

**Limitations:** do not load unrestricted native dynamic libraries or grant filesystem/network access
by default. Do not block a built-in theme/overlay feature on a plugin framework. A Rust-written app
does not by itself make third-party plugins safe.

**Acceptance:** a short threat model and decision ADR show the chosen extension model, dependencies,
license/unsafe review, and sandbox boundaries; if no option meets requirements, defer plugins.

**Verification:** prototype only with synthetic data; demonstrate resource exhaustion and denied
capability behavior. **Dependencies:** M10.3 and stable engine command API. **Estimate:** small spike.

## M11 — Focused editing and local AI-agent workflows

### M11.1 — One-track editor expansion evaluation

**Status:** first bounded slice implemented under M11.1a; further editing features remain proposed.

**User problem:** the user wants a more complete inbuilt editor in the spirit of Medal or a small
CapCut/Clipchamp workflow while keeping capture and library as the main product.

**Requirements:** first build and evaluate a bounded one-track prototype for arranging local clips,
cuts, simple transitions, text/input overlays, and basic audio placement/levels. Preserve the source
clips and export an ordinary video. Document undo, preview quality, export time, and exact supported
operations before deciding whether to continue.

**Affected code/platforms:** desktop editor UI, engine edit model, and local export pipeline on the
platforms that pass M8.1.

**Limitations:** no general-purpose multitrack NLE, community templates, cloud project format, or
ClipCraft-specific `.ccproj` requirement. Keep the current simple trim flow available and do not
delay core capture/platform work for advanced editing.

**Acceptance:** user testing shows the added workflow improves common clip preparation; edit math
uses integer ticks; project/session state is rebuildable or saved only if an ADR justifies it; export
preserves source files and meets measured performance limits. The decision may be to defer.

**Verification:** prototype screenshots/video, command-level edit tests, generated A/V sync tests,
and a written continue/defer decision. **Dependencies:** M8.1 and M10.3. **Estimate:** TBD after UX
spike.

### M11.1a — Small sequence editor: cuts, arrangement and local export

**Status:** implemented and software/virtual-display verified (2026-10-22); target hardware and
installed desktop portal checks remain open as documented in testing/gaps.

**Outcome / evidence:** quick trim currently exports only one range. The user requested a small
editor, reuse of FilmCraft editing features, and a reference layout with a left media browser,
large preview and bottom timeline. Build the portable single-track slice now; M8/M10 validation
remains required for broader platform or overlay claims, not a blocker for this local slice.

**Scope / requirements:** append library clips, select/scrub, split at the playhead, trim selected
source ranges, duplicate, reorder, ripple-delete and undo/redo. Keep source audio attached and
allow per-segment volume/mute. Reuse pinned FilmCraft track/cut algebra and media/codecs; preserve
quick trim. Export the ordered cuts as an ordinary MP4 on a bounded worker with progress/cancel.
Commands and CLI use the shared engine. Follow the reference's browser/preview/timeline hierarchy
with original controls/assets; no dead Effects/Text tabs. The editor has its own rail icon and
media library, Import clips, Send to editor from Clips, and Details/Add to timeline context menus.
Do not add per-thumbnail Add buttons or an artificial segment-count cap. Match the shared
charcoal/lime palette, embedded Inter font weights, compact controls and keyboard focus states.

**Limits:** one gapless video/audio sequence, at most 30 minutes, no separate segment-count cap, bounded
64-step/8 MiB history; edits are in-memory and disappear on app exit. No custom project format, transitions,
text, overlays, detached/multiple audio lanes, arbitrary effects, HDR or speed changes in this
slice. Export is SDR H.264/AAC, at most 720p/30 FPS, first source audio track only; normalize to
48 kHz stereo and preserve originals. Missing/unsupported media must fail with an actionable
message without replacing output; no required server or external encoder.

**Acceptance / verification:** typed command tests for exact cuts, history, bounds, invalid input
and immutable failed edits. Generated multi-clip fixtures prove order, cuts, duration, audio
continuity/mute, cancellation, no-overwrite collisions and source preservation. Inspect actual UI
at 1440×900; test keyboard actions and narrow/layout extremes. Run `cargo xtask ci` and docs gate.

**Docs:** README, gaps, roadmap, architecture, testing, editor guide and FilmCraft reuse ADR.
**Dependencies:** existing player/export stack; user authorizes this bounded slice before M8/M10.
**Estimate:** multi-part implementation.

### M11.2 — Local CLI and MCP footage-management adapter

**Status:** planned; the local Rust CLI exists, but an MCP adapter does not.

**Requirements:** extend `clipcraft-cli` with stable machine-readable JSON for library search, clip
inspection, trim/export, and safe metadata edits. Evaluate an MCP adapter over stdio that exposes the
same engine commands to a user-selected AI client and scopes operations to explicitly configured
library roots.

**Affected code/platforms:** `apps/clipcraft-cli`, `crates/clipcraft-engine`, and a possible Rust
MCP stdio adapter on Windows, Linux, and macOS.

**Limitations:** do not add an HTTP listener, remote control server, telemetry, or automatic footage
upload. ClipCraft tools must not send video/audio to an AI provider. Read-only tools are the default;
edits, delete, backup, and overwrite-sensitive operations require explicit user approval or an
explicit per-session policy. The host AI may have its own privacy/network behavior outside ClipCraft.

**Acceptance:** CLI and MCP use the same validated command registry; invalid paths and malformed
arguments are rejected; read-only mode cannot mutate files; mutation permission is visible and
revocable; outputs avoid leaking private paths unless explicitly requested.

**Verification:** protocol/schema tests, sandboxed temp-library tests, malicious path/oversize-input
tests, and an end-to-end stdio host test with synthetic clips. **Dependencies:** stable command IDs
and M9 backup permissions. **Estimate:** TBD.
