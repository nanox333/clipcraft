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
`.ccproj` or save-project workflow. Internal editor recovery is separately authorized under
M11.1c/ADR 0020; quick trim still keeps its active range in memory. Index loss must be recoverable
by rescanning folders.

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
MP4 without re-encoding, with progress/cancel and input preservation. Compression and size presets
are delivered under M4.3; sequence splitting/undo and text/effects/fades live in the separate M11.1a
editor. This entry describes the original quick-trim path, which remains available.

**Requirements:** support preview, frame-accurate start/end trim, split/cut, undo for the active edit,
and H.264/AAC MP4 export with progress/cancel.

**Limitations:** this quick-trim path has no multitrack timeline, effects, captions, overlays or
saved project. Use the separate editor for its supported text/effect operations; arbitrary native
plugins and remote render jobs remain outside scope. Preserve originals; export a new normal video file.

**Acceptance:** command-level tests reproduce the UI edit; generated fixtures verify duration,
frame-accurate cut points, audio sync, metadata, cancellation cleanup, and input-file preservation.

### M4.4 — GIF and loudness evening

**Status:** implemented and tested with generated clips on Linux; no Windows/macOS or real-game
evidence. Engine commands `clip.export_gif` and `clip.normalize`, the CLI
(`clip gif|normalize`) and the trim panel's output menu share one implementation. See [ADR 0018](adr/0018-gif-and-loudness-export.md).

**Requirements:** save a short range as a looping GIF; bring a range's sound to a common loudness without clipping.
Preserve originals; never overwrite; cancel leaves nothing behind; work offline.

**Limitations:** GIF is at most 30 s, 960 px wide, 20 fps and has no sound. Joining clips is not
included in quick trim; the separate M11.1a editor supports joins and simple fade dips.
Loudness evening needs AAC sound, applies one gain to every track, holds the gain back at
-1 dBTP, and re-encodes the sound (the picture is copied).

**Acceptance:** generated fixtures verify GIF size, frame count, delay and loop; normalised output measures at the target within 1 LU, keeps a
click within 50 ms of its place, never exceeds full scale, and decodes cleanly. Bad inputs,
cancellation and collisions leave no partial file and do not touch the sources.

**Open:** real-game and Windows evidence; speed on representative PCs (see
[testing.md](testing.md)); WebM/AV1 output. The separate editor's supported joins/fades are tracked
under M11.1a/M11.1b.

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
run, plus a one-minute native SDR 1080p60 throughput check. Signed Windows/macOS installers,
a two-hour 1080p60 soak, real screen reader, real-device audio, and broader
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
leases and CPU fallback. Generated-window AMD runs confirm zero CPU readbacks at matching dimensions,
including a one-minute native SDR 1080p60 throughput check (M5.2a). See
[the measurements](windows-performance.md). HDR/scaling, other GPUs, real-game frame times,
GPU/power comparisons and device-loss integration remain open. The two-hour Windows/AMD generated-window
1080p30 soak passed with eight drops, zero restarts and 24 decoded clips; a two-hour 1080p60 soak
remains open. Default HDR compatibility retains CPU conversion.
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

#### M5.2a — Isolate and remove the Windows 1080p60 throughput limit

**Status:** short generated-workload throughput criterion met on Windows/AMD SDR (verified
2026-10-23); broader diagnosis and hardware gates remain open. Nonblocking cached reads and a
half-period SDR eligibility request remove a repeatable roughly 55-fresh-frame/s capture limit.
The 60.026-second run returned 3,589 pictures (0.333% deficit), with 3,602 fresh reads, no repeats,
zero readbacks/restarts and 177.10 MiB peak sampled RSS. Saved output fully decodes. The historical
32 fps MF input-wait stall stopped reproducing before functional changes and remains unexplained.
The two-hour soak still covers 1080p30 only. See [evidence and limits](windows-performance.md).

**User outcome:** supported Windows hardware captures a generated 1080p60 source without a
half-rate encoder bottleneck, while bounded replay, safe clip saving and CPU fallback remain intact.

**Scope and evidence:** isolate direct generated RGBA input to the hardware encoder from WGC,
audio, UI and recorder scheduling; count actual output pictures through drain and independently
decode the MP4. Record requested/negotiated rates, driver macroblock capability and stream changes.
Compare the same generated workload before/after any fix. Hardware APIs and unsafe code remain
inside the existing platform boundary; no dependency or vendor-wide support claim is implied.

**Acceptance:** explain the measured bottleneck; preserve requested stream geometry/rate across
renegotiation or return a fallback error; retain bounded failure/reordering/output-accounting tests.
For a 1080p60 throughput pass, measure at least 60 seconds with under 1% dropped pictures and under
1% deficits in both fresh reads and actual returned pictures against the requested cadence, no native
CPU readbacks, bounded memory and fully decoded saved output. Short encoder-only throughput alone
does not establish a capture pass or real-game impact. Run `cargo xtask ci` and documentation checks.

**Documentation/dependencies:** update README, gaps, Windows performance evidence and ADR 0014 as
needed in the same change. Depends on the implemented M5.2 native path and existing hardware;
estimate depends on isolation results. Device removal, HDR/scaling and other GPUs remain separate.

### M5.3 — Continuous offline capture-to-export workflow verification

**Status:** implemented and verified with generated Linux/X11 media (2026-10-23), following M11.1e.
The full gate passes 535 tests across 33 targets. GitHub: [#4](https://github.com/nanox333/clipcraft/issues/4).

**Problem / evidence:** individual capture, game arming, library, editor, export and recovery slices
have evidence, but their complete connected workflow must be checked on the current revision before
making an alpha claim. Windows hardware is unavailable; generated Linux/X11 evidence is feasible.

**Scope:** add a repeatable pure-Rust integration scenario using production capture/save, library,
player, editor and recovery APIs. Run the actual native app on an isolated display/data folder with
a generated game process/window. Check detection → save → metadata/thumbnail/playback → cuts/text →
export → restart recovery. Exercise cancellation, missing sources, destination collisions and
interrupted jobs. Fix reproducible failures with focused regressions. Audit local workflow network calls.

**Limitations:** no new editor features, servers/accounts, uploads, dependencies, codecs, platform
promises or performance claims. Generated audio/window evidence does not prove physical devices,
anti-cheat, real games, Wayland or Windows/macOS. Preserve existing media and bound all fixtures,
worker waits, caches and output. If network isolation cannot be enabled here, distinguish observed
no-network use from a physically disconnected or network-disabled release test.

**Acceptance:** one connected automated scenario captures real encoded pictures and generated sound,
retains the captured game and user metadata through reload/rescan, generates a thumbnail, plays saved
and exported media, edits/cuts/text through shared commands, restores a paused history-free draft,
and reports safe failure for missing source/cancellation/collision without altering originals.
Native process/window detection, save hotkey, library/editor actions, export, normal and interrupted
restart are documented with screenshots and exact local/unsupported evidence. `cargo xtask ci` and
the documentation gate pass; existing Windows/hardware release gates remain open.

**Verification / documentation:** portable synthetic integration test, native Linux/X11 session with
generated media and optional external decode oracle/network syscall audit. Update README, roadmap,
gaps, testing, release checklist and architecture guidance if behavior changes. No private footage
or machine paths enter Git. **Dependencies / estimate:** M7.1b and M11.1a–e; one verification/fix cycle.

**Result:** the connected test exposed AAC-padding duration disagreement between import and
playback/export. Native forced termination exposed a partial file blocking same-title retry. Trim
failure review additionally found truncating temporary creation and a raced rename that could
replace existing files. All three now have failing-before/passing-after regressions. Import uses
bounded reader metadata; Send to editor resolves it off the UI thread; automatic names reserve
temporary/recovery paths; trim uses exclusive ownership/no-replace hard-link publication and a
final cancellation check. No source or occupied temporary is deleted to make a retry work.
Native virtual audio, playback, edits, retry and normal/crash recovery pass; `strace` observed no
Internet-addressed calls across five sessions. Namespace creation was unavailable, so physical
network disconnection and Windows/hardware release gates remain open. See `testing.md`.

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

### M7.3 — Optional local game data for CS2, Dota 2 and League

**Issue:** [#5](https://github.com/nanox333/clipcraft/issues/5). **Status:** adapters and setup/status
UI/CLI implemented; portable protocol/socket/keyboard checks and Windows compilation verified;
native live-game/League TLS evidence remains open.

**Problem/evidence:** process recognition cannot provide map, match phase or local-player stats.
The maintainer requested these integrations using public Valve GSI/Dota configuration and Riot
Live Client API documentation. See [references/setup](game-integrations.md).

**Scope/requirements:** engine `game_state.parse`/`game_state.install`, existing recording settings,
one Rust platform worker and Games UI/CLI. Explicit opt-in, transient allowlisted snapshots,
loopback-only authenticated Valve listeners, bounded input/IO/work, OS-verified local League TLS.
Show disabled/waiting/live/error states and setup instructions. Capture stays independent.

**Limitations:** no hosted service, account, injected code, memory reading, remote stats, upload,
Discord publication, automatic highlights or rich saved-clip metadata. League transport requires
Windows/macOS. Config/key loss, occupied ports and game patches can require explicit setup/retry.
Existing configs/media/partials must be preserved. ADR 0022 scopes the optional local listener.

**Acceptance:** default-off settings; exclusive idempotent setup; authenticated game-ID validation;
local-player fields without retained identity; stale/error clearing; bounded HTTP/body/IO and
snapshots; keyboard switches and shared CLI commands; failures cannot change capture policy.
Success and malformed/collision/shutdown lifecycle cases have tests. Synthetic messages and
compilation cannot establish live-game compatibility.

**Verification:** engine/platform/UI tests, native Linux before/after screenshots and generated
authenticated loopback traffic, `cargo xtask ci`, docs gate and Windows target check. Actual
CS2/Dota/League sessions, League native TLS and macOS runtime checks stay open until run.
**Docs:** README, gaps, roadmap, architecture/testing/settings, game-integrations, ADR and attribution.
**Dependencies/estimate:** existing M7.1 settings/detection; one implementation plus separate
target-OS live-match verification. Overwatch/Deadlock rich adapters remain research.

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

**Issue:** [#6](https://github.com/nanox333/clipcraft/issues/6). **Status:** Rust policy, IPC worker,
General settings/CLI and mock verification implemented. The maintainer-supplied shared identity and
basic Windows IPC activity acknowledgment are verified; artwork/profile visibility, live games
and macOS runtime still need live checks.

**Requirements:** provide opt-in local Rich Presence that can show ClipCraft activity and, only if
the user enables it, the captured game name and matching fresh map/mode/team score/hero information.
Explain exact fields before enabling; clear old activity on disabled/reduced consent, lost game or
fresh match data, application-ID change and exit. Basic labels are clipping/editing/browsing.
Use shared `presence.preview` policy and existing settings commands for UI/CLI; one replaceable
target, bounded framing/IO/retry, acknowledgments and conservative update limits.

**Affected code/platforms:** desktop app and a small Rust presence adapter on Windows/Linux/macOS;
Discord's local IPC availability is optional.

**Limitations:** no clip upload, link, Discord login inside ClipCraft, friends/community surface, or
collection of Discord account data. Discord availability is optional and must not affect capture.
Only the local desktop IPC is used; Discord may publish opted-in activity through its service.
No clip paths/titles, media, health/money/KDA, join secrets or raw game updates. Without a valid
effective shared/override public Application ID, show unconfigured state and perform no IPC discovery. No bot/token.
Clear/exit is bounded and best effort for unresponsive Discord; disconnect is the fallback.

**Acceptance:** default is off; enabling/disabling and Discord unavailable states are clear; stopping
ClipCraft clears the presence; capture remains functional when Discord is missing or blocked.

**Verification:** engine disclosure/hostile-input matrix; actual Unix mock handshake/set/clear/ping,
nonce/error/oversize/timeouts, coalescing/rate/privacy/reconnect/ID-change/late-READY/shutdown tests;
native settings screenshots/keyboard and mock IPC, full gate and Windows/macOS compile checks.
M10.2c now has owned-identity Windows IPC and actual profile-art evidence. Live-game context,
other-account audience and broader platform checks remain open.
**Documentation:** README, gaps/roadmap, architecture/testing/settings/release, Discord guide and
ADR 0023 ship together. **Dependencies:** M7.1 identity and M7.3 match adapters. **Estimate:** one
prototype plus independent live Discord/platform verification.

### M10.2a — Medal-inspired presence and automatic end-user setup

**Issue:** [#7](https://github.com/nanox333/clipcraft/issues/7). **Status:** implementation complete;
maintainer-supplied public identity configured 2026-10-24; M10.2c uploads CUT and verifies actual
Windows game/CUT artwork. Other-account audience and broader platform evidence remain open.
Ordinary builds and empty release variables use the product ID; valid explicit overrides remain
supported. Regression tests cover missing/empty build values and malformed overrides. Windows IPC
empty reads now wait for data; a deterministic empty/data/disconnect regression and real Discord
READY/activity acknowledgment pass. Profile artwork/audience and macOS remain open.
**Requirements:** normal users enable one toggle; maintainer supplies a shared public build ID.
Keep saved custom IDs in General's advanced section with Apply/reset. Show a local preview without
connecting; use `Clipping <game>`, `with ClipCraft`, optional readable match details and original
large game/small ClipCraft badges. Preview/CLI share `presence.card` disclosure policy.
**Limitations:** no borrowed ID/logos, elapsed timestamp, uploads, game art or per-game application
identity spoofing. Shared setup requires one Discord portal registration and asset upload; no ID
is fabricated. Unknown games use the generic ClipCraft badge. Existing privacy/IPC limits remain.
**Acceptance:** preview works with no ID; off does not connect; normal UI has no ID form; advanced
Apply/reset works by keyboard; resolver validates and prioritizes override/shared IDs; assets are
bounded and never reveal a game without consent; exporter refuses an existing directory.
**Verification:** policy/art and UI keyboard tests, actual Unix mock IPC, native Linux screenshots,
full workspace gate and cross-platform compile checks; real Discord/client/Windows/mac runtime open.
**Affected:** engine, egui, CLI, release workflow, documentation. No dependency or transport change.

### M10.2b — Establish the selected CUT identity

**Status:** implemented and locally verified 2026-10-24; CUT is uploaded and Windows profile
artwork is verified by M10.2c.

**Problem / outcome:** the maintainer selected concept A; replace the generic diamond with a
consistent original C/play identity and deliver reusable artwork.

**Scope:** shared bounded Rust renderer, native window/header, Discord product badge, SVG/PNG
masters, small icons, light/dark wordmark lockups and usage/provenance documentation.

**Limitations:** no new dependency, network requirement or capture change. Publisher game logos
and Discord portal uploads are separate work; legacy game placeholders remain.

**Acceptance:** identical silhouette across exports and app; visible stem/opening at 16–64 px;
invalid dimensions rejected before allocation; exporters refuse an existing destination;
fonts/assets have attribution; actual desktop screenshot and keyboard navigation checks pass;
`cargo xtask ci` passes. Preserve unrelated local settings edits.

**Verification:** brand geometry/hostile-size tests, presence badge tests, asset export and
overwrite refusal, native 1440×900 desktop screenshots and Tab/Shift+Tab checks passed.
`cargo xtask ci` passed, including the texture-cache lock regression. See [brand guide](brand.md)
and [testing evidence](testing.md).

### M10.2c — Discord-hosted game artwork and branded release

**Status:** implemented; all three Windows profile artworks, live IPC, full CI and release builds
verified 2026-10-24. Other-account audience and broader platform checks remain open.

**Problem / outcome:** replace the rejected typographic game badges with actual Discord-hosted
CS2, Dota 2 and League icons plus the small CUT product badge.

**Scope:** fixed public CDN references selected by the existing game-name consent; no bundled
publisher image files, new dependency, HTTP client or runtime discovery endpoint. Remove the
legacy game asset exporter output. Upload CUT under the owned application and build the release.

**Limitations:** other games and the offline preview use CUT. Fixed CDN hashes can become stale;
Discord controls fetching/rendering. A protocol ACK alone does not verify displayed artwork.

**Acceptance / verification:** all three fixed references pass policy and live IPC checks;
game-name consent off and editing/browsing omit game references; arbitrary URLs/hostile labels
cannot select remote images; owned application identity remains unchanged; exporter writes only
the original product PNG and license, refusing existing output; native preview/keyboard and
actual Discord artwork are inspected; `cargo xtask ci` and release builds pass.

**Docs:** README, gaps, presence guide, architecture, release/testing evidence and ADR 0023.

**Evidence:** CUT icon and ClipCraft name saved in the owned portal. Real CS2/Dota/League cards
displayed game art and the small CUT badge after the maintainer enabled Discord activity sharing.
Screenshots and labelled-preview logs remain in ignored `target/discord-art-evidence/`.

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
allow per-segment volume/mute, colour looks and black/white fades. Independent text layers support
placement, timing, styling, fades, split and duplicate. Reuse pinned FilmCraft track/cut algebra,
text rendering and media/codecs; preserve quick trim. Export the ordered cuts as an ordinary MP4 on a bounded worker with progress/cancel.
Commands and CLI use the shared engine. Follow the reference's browser/preview/timeline hierarchy
with original controls/assets; no dead Effects/Text tabs. The editor has its own rail icon and
media library, Import clips, Send to editor from Clips, and Details/Add to timeline context menus.
Do not add per-thumbnail Add buttons or an artificial segment-count cap. Match the shared
charcoal/lime palette, embedded Inter font weights, compact controls and keyboard focus states.

**Limits:** one gapless video/audio sequence plus bounded text layers, at most 30 minutes, no separate
segment-count cap, bounded 64-step/8 MiB history. M11.1c supersedes this slice's original
session-only lifetime with internal recovery, without adding a project-file workflow.
Simple fade dips, colour looks and text are built. No custom project format, cross-dissolves,
overlapping video, input overlays, detached/multiple audio lanes, arbitrary effects, HDR or speed
changes in this slice. M11.1b adds shared preview rendering and size presets. This slice initially exported SDR H.264/AAC at most 720p/30 FPS; M11.1d supersedes that
quality limit. The first source audio track is used; normalize to
48 kHz stereo and preserve originals. Missing/unsupported media must fail with an actionable
message without replacing output; no required server or external encoder.

**Acceptance / verification:** typed command tests for exact cuts, history, bounds, invalid input
and immutable failed edits. Generated multi-clip fixtures prove order, cuts, duration, audio
continuity/mute, cancellation, no-overwrite collisions and source preservation. Inspect actual UI
at 1440×900; test keyboard actions and narrow/layout extremes. Run `cargo xtask ci` and docs gate.

**Docs:** README, gaps, roadmap, architecture, testing, editor guide and FilmCraft reuse ADR.
**Dependencies:** existing player/export stack; user authorizes this bounded slice before M8/M10.
**Estimate:** multi-part implementation.

### M11.1b — Preview/export consistency and editor file-size presets

**Status:** implemented; software/fake-provider and Linux virtual-display checks pass (2026-10-22).
Real-device encoder/audio validation remains open.

**Problem / evidence:** a one-second default 12 fps GIF was measured as 0.96 seconds. Editor audio
fades were applied during export only; preview text used a different weight/layout/shadow than
export. The editor lacked the file-size choices already present in quick trim, and roadmap/gap
text contradicted the text/effects features in the current code.

**Scope / requirements:** preserve GIF duration through integer centisecond scheduling, including
partial final frames. Use shared bounded FilmCraft text rasterization and matching audio fade gain
in preview/export. Add Standard, estimated 10 MB, 25 MB and custom 1–500 MB editor MP4 exports,
with the same typed engine command and bounded JSON CLI transport. Reserve encoded AAC and mux
headroom, reject impossible budgets, report actual size/settings/encoder, and try the configured
GPU encoder with a safe software fallback. Keep progress/cancel and source preservation.

**Limitations:** pure Rust and offline; no new hosted service, project format, codec or capture
platform claim. Size targets are estimates, not Discord limits. This slice initially capped output at
720p/30; M11.1d adds explicit bounded resolution/FPS choices; small budgets can reduce dimensions. One gapless video/audio sequence plus text
layers; text, layer count, masks, history and workers remain bounded. Real-GPU export and physical
audio/Windows validation are separate evidence gates.

**Acceptance / verification:** decoded GIF durations match requested ranges to centisecond
precision at all supported frame rates; preview/export share text pixels/placement and fade math;
representative text/cut/effect/mute sequences export under each preset with correct duration/audio.
Invalid/custom/impossible budgets, cancellation, hardware refusal/midstream failure and output
collisions preserve all source files. Check keyboard and actual desktop UI screenshots. Run
`cargo xtask ci` and `cargo xtask docs-check`.

**Docs:** README, gaps, roadmap, editor guide, architecture, testing and relevant ADRs. Correct stale
feature exclusions rather than documenting existing text/effects as planned.
**Dependencies:** M11.1a and existing clip.export preset/hardware seam; no hardware needed for the
software/fake-provider acceptance slice. **Estimate:** one focused implementation/review cycle.

### M11.1c — Timeline media, prepared playback and internal draft recovery

**Status:** implemented and software-complete for this slice (2026-10-22). The final gate passes
520 tests across 30 targets plus strict lints/layers/assets/licenses; Linux virtual-display
timeline, zoom/pan, restart, missing-media, corrupt/discard and normal-exit checks pass. Additional
shutdown-race regressions and independent review pass. Hardware/platform evidence remains separate.
See [testing](testing.md).

**Problem / evidence:** the timeline repeated one library thumbnail and showed an undecoded audio
lane. Opening the next source at each cut could interrupt preview; closing the application lost
the edit. The user approved real timeline media, prepared playback across cuts and local recovery
after the M11.1b export work was merged.

**Affected code/platforms:** `clipcraft-player` analysis/player lifecycle, `clipcraft-engine`
commands/recovery, `apps/clipcraft` timeline/playback/recovery UI and `apps/clipcraft-cli` JSON
transport. The software slice is portable Rust; native evidence in this environment is Linux/Xvfb
only, with existing media/OS support limits unchanged.

**Scope / requirements:** analyze visible source ranges through a shared typed `editor.timeline`
command; paint source-time thumbnail strips and first-track audio envelopes with trim, split and
zoom mapping. Keep analysis, file access and decoding off the UI thread, with loading/error states.
Prepare the next source paused, without opening its audio output until promotion. Retain cancelled
decoders in the playback budget until shutdown is acknowledged. Save validated current cuts, text,
imported references, title, playhead and export preset as internal local recovery metadata; restore
paused on launch with a visible notice. Expose shared `editor.draft.load`, `.save` and `.discard`
commands to the bounded JSON CLI. Allow explicit discard and actionable retry; preserve corrupt or
future-version records until the user discards them. Source videos and exports remain unchanged.

**Limitations:** pure Rust, offline, no project picker, `.ccproj`, server or upload. One analysis
accepts 1–12 160×90 thumbnail frames and 16–2048 signed min/max audio buckets, an interval of at
most 30 minutes, and a source of at most 24 hours. Audio envelopes use the first track, streaming
MP4/MOV native-rate packets; other containers use an 8 kHz fallback limited to ten seconds and
a nominal native PCM span of 16 MiB, otherwise report unsupported. These envelopes are a
visualization, not a separate editable audio track. Before opening MP4 sources, cap total movie/
fragment metadata at 16 MiB, box traversal at 100,000/eight levels and declared sample entries at
one million across all source tracks; declared encoded samples are at most 64 MiB before probing.
Long high-rate PCM sources can exceed that budget even
for short selections. Check packet size before decoding and ALAC configuration before construction;
validate returned scalar counts without claiming a hard codec-scratch bound. See architecture
for codec-specific limits. The UI uses one analysis worker,
eight queued requests, a 32 MiB/256-entry cache and at most two texture uploads per poll. These
payload/cache limits do not describe total codec memory. At most two current/prepared/retiring
players exist; slow decoding or cancelled workers can still cause a pause. Recovery has a 16 MiB
metadata limit, a 500 ms debounce and a bounded two-second normal-exit flush; the final unsaved
input can be lost on a crash or stalled filesystem. Undo/redo history is not restored. Missing
media remains a reference and must be available for playback/export. No capture/platform or
reference-machine performance claim is added.

**Acceptance:** distinct generated source frames appear in the proper trimmed timeline range;
waveform peaks reflect real first-track audio and silence/mute states without blocking controls.
Reorder, seek and repeated close/open cannot grow decoder or analysis workers without bound.
Prepared playback promotes the correct source/frame and does not emit its audio before the cut.
Restart restores edits/text/preset/playhead paused with fresh undo history; explicit discard does
not resurrect a queued save. Corrupt/oversized/unknown recovery and failed writes preserve the
previous file and sources, with a visible remedy. Missing sources do not erase the edit.

**Verification:** generated Rust video/audio fixtures exercise exact sample positions, envelopes,
no-audio, cancellation, hostile ranges/dimensions and payload bounds; cache tests cover queue,
fingerprint invalidation and resource limits. Playback tests cover promotion and cancellation
retirement. Draft tests cover round trips, missing references, malformed IDs/history, version and
size limits, atomic preservation, symlinks and repeated discard. Exercise CLI analysis and draft
commands. Inspect native 1440×900 before/after screenshots, keyboard interactions and restart/
discard behavior using generated media. Run `cargo xtask ci` and `cargo xtask docs-check`; record
actual results in testing/gaps. Hardware capture/audio tests remain separate.

**Documentation impact:** README, gaps, roadmap, PLAN, editor guide, architecture, testing and
[ADR 0020](adr/0020-internal-editor-recovery.md). **Dependencies:** M11.1a/M11.1b and existing
FilmCraft readers/player; no hardware dependency for this software slice. **Estimate:** one
focused implementation/review cycle, with slow-source decoder behavior the main resource risk.

### M11.1d — Editor export resolution and frame rate

**Status:** implemented; 527 tests across 31 targets and Linux virtual-display checks pass
(2026-10-23). [GitHub #2](https://github.com/nanox333/clipcraft/issues/2).

**Problem / evidence:** M11.1c editor exports always render at at most 720p/30 FPS, including recordings captured at higher resolution or frame rate. The user approved higher quality export while preserving the simple local workflow.

**Scope / requirements:** extend the Rust sequence export, shared `editor.export` JSON command, export dialog and internal draft recovery with 720p, 1080p and source-resolution choices plus 30/60 FPS. Existing requests/drafts default to 720p/30. Source follows the first clip's aspect ratio, bounded to a 3840×2160 landscape or 2160×3840 portrait canvas without enlarging source pictures. Mixed clips fit with black bars. Existing 720p/1080p choices retain a 16:9 canvas. File-size presets may reduce resolution/bitrate; show actual dimensions/rate and explain reductions. Report actual FPS/bitrate and recover the selected options after restart.

**Affected platforms/code:** portable export/engine crates, native desktop editor and JSON CLI. Linux virtual-display and generated media verification here; physical GPU, Windows/macOS throughput and audio devices remain unverified.

**Limitations:** pure Rust, fully offline, no new runtime dependency or server. Existing SDR H.264/AAC input, stereo AAC output, first audio track, 30-minute duration, no source overwrite and cancellation rules remain. Maximum output is 8,294,400 pixels and 60 FPS; 30 FPS editing-grid boundaries remain unchanged. No new codecs, source-FPS auto-selection, interpolation, HDR, quick-trim changes or general editor expansion. Hardware refusal must safely retry software with identical settings. Bitrate/size targets are estimates, not guarantees.

**Acceptance:** legacy payloads retain previous defaults. Chosen resolution/FPS reach encoder, packet timestamps, mux, CLI report, text preview and recovery. 60 FPS cuts do not accumulate audio drift; final partial video samples preserve exact duration. Small sources do not upscale; portrait/ultrawide source canvases preserve aspect ratio. Tight size presets visibly reduce resolution. Unknown enums, unsupported FPS, hostile sizes/ranges, failed encoders, cancellation and existing destinations preserve original files and valid recovery data.

**Verification:** generated Rust H.264/AAC fixtures and external FFmpeg/FFprobe oracles for 30/60 FPS motion, fractional cuts and continuous audio, real higher-resolution exports, plans for 4K/portrait/ultrawide and size reduction; hardware stand-ins and failure/cancel tests; engine JSON/draft compatibility and recovery tests; native before/after screenshots and keyboard checks. Run `cargo xtask ci` and `cargo xtask docs-check dfe99f3`.

**Documentation:** update README, gaps, roadmap, backlog, editor guide, architecture, testing and relevant ADRs in the same change.

**Dependencies / estimate:** M11.1a–c and existing FilmCraft encoders. One focused implementation and verification cycle; no hardware prerequisite for this software slice.

### M11.1e — Editor export performance and responsiveness

**Status:** implemented (2026-10-23), following M11.1d. The full gate passes 532 tests across
32 targets; the manual composition benchmark is deliberately ignored in CI. Measured software
exports and native Linux verification are recorded in `performance.md` and `testing.md`.
GitHub: [#3](https://github.com/nanox333/clipcraft/issues/3).

**Problem / evidence:** before this change, a matching-size export ran every RGBA picture through
the thumbnail filter, allocated a black canvas and overlaid it. Background export also disabled
keyboard transport while mouse playback remained available. Profile these paths before changing them.

**Scope / requirements:** reduce redundant frame allocations/copies without changing decoded pixels,
timing, codec settings, text/fades, output planning or atomic publication. Keep one export worker,
bounded scratch storage and cooperative cancellation. Allow preview transport during export while
keeping edit mutations and a second export disabled; limit progress-triggered UI wakeups.

**Limitations:** pure Rust, offline, no dependencies, no capture/provider changes, no frame queue,
no codec quality tradeoffs or speed promises on unmeasured hardware. Codec calls cannot be interrupted
mid-call. Physical GPU, real-game interference, Windows and macOS performance remain unverified.

**Acceptance:** a repeatable Rust benchmark records the machine, workload, wall time and available
process-memory evidence against `3b6642a`; the matching-size composition path removes two frame-sized
temporary allocations and measures faster without changing pixels. Mixed sizes, alpha, effects,
fades and text preserve reference results; 30/60 FPS audio/timestamp tests pass. Export progress wakes
at most ten times per second plus completion; cancellation remains actionable and removes partials.
Keyboard Space/stepping work during export, mutations/duplicate exports stay blocked, and a disconnected
worker returns an error. Progress/cancellation stay visible and clickable in the top toolbar at a
960×640 desktop size. Verify native playback/cancel/completion and screenshots, then `cargo xtask ci`.

**Verification / documentation:** deterministic frame-reference and worker/keyboard regressions;
generated 1080p60 exports before/after with identical settings and repeated runs, plus native Linux
virtual-display checks. Update README, gaps, roadmap, editor, architecture, testing and performance
guidance in this change. Timing thresholds are manual evidence, not flaky CI assertions.

**Dependencies / estimate:** M11.1a–d, existing FilmCraft codecs and export job. One focused measured
implementation cycle; hardware release gates remain separate.

### M11.1f — Bounded playback delivery and superseded seeks

**Issue:** [#9](https://github.com/nanox333/clipcraft/issues/9). **Status:** implemented; portable
verification below, no hardware speed claim. Follows the user-requested Concat review (2026-10-25).
**Problem/evidence:** `ba03a60` used an unbounded decoded-picture event channel, so a stalled UI
retained historical RGBA pictures; an in-flight old seek could publish after a new request.
**Requirements:** one pending latest picture, reliable opening/end/failure slots, atomic revision
invalidation/enqueue/publication for seek/step/replay; nonblocking capacity behavior and stop;
check native picture dimensions before conversion. Existing playback API/engine operations stay.
**Limitations:** no global playback/thumbnail/waveform scheduler, prefetch/proxy/GPU feature, codec
interruption, control/audio channel redesign or total RSS promise. No Concat source port, FFmpeg,
new dependency, unsafe or licensing change. Pure Rust/offline/source-preservation rules apply.
**Acceptance:** stalled poll returns at most opening/latest picture/terminal in order; final picture
precedes Ended; stale results are dropped while fresh playback continues; failures/start errors
are visible; close/poison/revision overflow fail closed; malformed/oversize dimensions are refused;
audio/loop/step/seek and prepared-source transitions still pass.
**Verification:** deterministic burst, controlled-thread race, concurrent-producer ordering,
lifecycle/close/failure/poison/overflow/bounds tests; generated MP4 unpolled through end; full
workspace gate and documentation check. Native playback smoke is supplementary, not a GPU claim.
**Affected:** `clipcraft-player`, existing desktop consumers. No UI layout/new command operation.
**Docs:** README/gaps, roadmap, architecture/performance/testing and ADR 0024 ship together.
**Dependencies/estimate:** existing player only; one focused portable slice. Follow-ups: coordinate
bounded worker priorities/caches, then evaluate a small independent GPU compositor and proxies.

### M11.1g — Cooperative playback-first decode admission

**Issue:** [#10](https://github.com/nanox333/clipcraft/issues/10). **Status:** implemented; portable
checks documented below, hardware latency/speed measurements open.
**Problem/evidence:** M11.1f bounded results but each decoder caller still ran independently.
FilmCraft's background hint changes decoder retention, not admission priority across jobs.
**Requirements:** a shared coordinator admits <=3 stages, <=1 background stage and <=64 waiters;
at most 48 waiters are background, reserving 16 registrations for foreground. Queued foreground
wins; cancellation/unwind releases registration and permits. Gate worker-side
player open/video and audio as foreground, thumbnail/timeline pictures/waveforms as background.
Release background admission between analysis chunks, and audio admission before output waits.
**Limitations:** logical slots are not CPU/codec-thread or memory reservations; no preemption of an
active codec/file call, full job/decoder pool, proxy, GPU/export/capture change or hardware claim.
No new UI-blocking gate on cached JPEG/source-info paths; thumbnail's old API has no cancellation
argument. Capacity/poison use existing failure states. No dependency, unsafe or Concat source port;
Rust-only, offline, exact timing, source-preservation and existing engine/API contracts remain.
**Acceptance:** deterministic priority and reserved-capacity checks, cancellation/overflow-capacity/
poison/panic paths release correctly; active/background counters stay bounded under concurrency;
generated playback+sound, thumbnails and waveform analysis complete together without source changes;
audio/seek/loop/prepared editor/failure/cancellation tests and full quality/docs gates pass.
**Affected:** `clipcraft-player` and existing engine callers; no command ID/schema/UI layout change.
**Verification:** isolated coordinator tests and a generated MP4 integration; no real device/GPU run.
**Docs:** README/gaps, roadmap, architecture/performance/testing, ADR 0025. **Dependency/estimate:**
M11.1f/existing workers, one focused portable slice; pooled decoder reuse/GPU/proxies remain later.

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
