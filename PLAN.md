# ClipCraft plan

**Status:** early build; M0 workspace and first local-library slice implemented
**Date:** 2026-10-19
**Product:** a local-first instant-clip recorder, clip library, and basic cut tool for players

**Implementation requirement: pure Rust.** First-party product code, UI, CLI, and required tooling
belong in a Rust Cargo workspace. No JavaScript/TypeScript frontend or non-Rust app runtime.
**Hosting requirement: none.** The app must not need a hosted backend, account, network connection,
or server process for capture, library, editing, settings, or export.

## Product direction

ClipCraft should make the path from “that just happened” to a clip saved on the user's computer
take one hotkey and a few seconds. The core loop is:

```text
choose capture source → keep a bounded replay buffer → press the clip hotkey
→ save the last N seconds (+ optional post-roll) → make a quick cut or trim → export locally
```

Capture and the local clip library are the product; the editor is a quick trim/cut tool, not a
general-purpose non-linear editor or social network. ClipCraft is a standalone offline desktop app: capture,
library, editing, settings, and export must work without an account, network connection, or hosted
server. It keeps recordings on the user's computer and exports ordinary video files. Sharing means
the user chooses what to do with an exported file outside ClipCraft.

[Medal.tv](https://medal.tv/) is the feature reference for the loop: instant replay, configurable
hotkeys and durations, game/window capture, system audio plus microphone, a clip library, quick
edits, and local storage controls. It is inspiration for behaviour and workflows only. ClipCraft will
recreate the compact dark capture-first look as closely as possible in layout, palette, sizing,
hierarchy, control density, library cards, editor arrangement, and lime action/selection states.
ClipCraft will use its own name, icons, labels, and visual assets, and will not use Medal's logo,
source code, network protocol, or proprietary media.

## Pure-Rust implementation and repository practice

ClipCraft is a clean-room Rust rebuild. The desktop app, UI, capture and editing engines, CLI,
local library metadata, and automation are implemented in Rust in a Cargo workspace. Use a native
Rust UI (initially egui/eframe); do not add a JavaScript/TypeScript app, Electron or Tauri shell,
hosted backend, or bundled FFmpeg runtime. The app must not need a server process to run. Choose
permissively licensed Rust dependencies and implement formats from public specifications.
FFmpeg/ffprobe may be used as external fixture generators or test oracles, never linked or shipped.
Platform capture and hardware APIs sit behind a small Rust platform crate and safe `Result`-returning
interfaces; isolate and document any necessary OS FFI there, with a pure-Rust fallback wherever
practical.

Follow the ArtCraft documentation model and tailor it to ClipCraft:

- `AGENTS.md` is the short, authoritative set of rules for all contributors: never crash or lose
  work, no panic shortcuts in non-test code, hostile-input bounds, clean-room/licence rules, and
  the limited platform-FFI boundary.
- `CLAUDE.md` is the working guide: read `AGENTS.md`, then the current roadmap/gaps, architecture,
  and relevant crate docs; keep work autonomous and follow the repository's single quality gate.
- `README.md` explains the product, Rust-only stack, current status, setup, run/test commands,
  architecture links, and original ClipCraft screenshots.
- Public planning and engineering references live in `ROADMAP.md` and `docs/` (`architecture`,
  `agents`, `backlog`, `contributing`, `testing`, `gaps`, issue requirements, and short ADRs). Keep
  detailed working notes and Medal reference screenshots under a gitignored local `plan/medal/`
  directory.
- Give milestones and tasks stable IDs. Use one `cargo xtask ci` entry point for formatting,
  linting, tests, dependency/layer checks, and platform checks; CI and contributor instructions call
  the same gate. Keep the engine command registry shared by the UI and CLI so features can be driven
  and tested without clicking through the app or starting a control server.

The product model and interaction patterns may be inspired by ArtCraft siblings; do not copy their
source code. Reuse a sibling crate only after checking its license, API fit, and maintenance cost.

## Principles taken from the ArtCraft repositories

| Reference | Adopt | Adapt for ClipCraft |
| --- | --- | --- |
| [EffectCraft](https://github.com/storytold/effectcraft) | engine-first design, exact time math, render jobs, CLI automation, evidence-based issue workflow | keep the editor intentionally small; prioritize capture latency and bounded memory |
| [PhotoCraft](https://github.com/storytold/photocraft) | thin egui shell, one command registry for UI and CLI, versioned local metadata, no-panic input handling, contributor/agent guidance | keep the UI/CLI command model and no-panic rules; do not make a custom edit-project format the center of a clip recorder |
| [FilmCraft](https://github.com/storytold/filmcraft) (sibling video project) | reusable media/frame/audio abstractions, rational timebase, codecs, export jobs, platform hardware adapters | extract or depend on shared crates only after an API review; do not fork a large editor wholesale |
| Medal.tv | one-key capture, replay buffer, clip library, quick editing, local storage controls | keep the familiar capture-to-export loop; omit cloud, social, account, and link features |

Use the public [EffectCraft `AGENTS.md`](https://github.com/storytold/effectcraft/blob/main/AGENTS.md),
[`CLAUDE.md`](https://github.com/storytold/effectcraft/blob/main/CLAUDE.md), and
[README](https://github.com/storytold/effectcraft/blob/main/README.md), alongside the corresponding
[PhotoCraft `AGENTS.md`](https://github.com/storytold/photocraft/blob/main/AGENTS.md),
[README](https://github.com/storytold/photocraft/blob/main/README.md),
[`docs/contributing.md`](https://github.com/storytold/photocraft/blob/main/docs/contributing.md),
and [`docs/architecture.md`](https://github.com/storytold/photocraft/blob/main/docs/architecture.md)
as concrete examples for contributor rules, agent workflow, issue requirements, and product/setup
documentation described below.

The implementation is clean-room: study public behaviour and specifications, use permissively
licensed dependencies, and keep third-party assets and their attribution sidecars explicit.

## Medal visual references and direction

The supplied Medal screenshots refine the target. The library is a dense four-column thumbnail grid:
each card has a duration badge, bold clip title, compact game/device/file-size/date metadata, and a
bottom action row. The top capture strip shows game detection, hotkey, replay duration, recording
state, long-recording mode, and resolution/frame rate. The settings examples prioritize PC storage
path, a local storage limit, and a segmented disk-usage bar; the recording panel has a clear capture
toggle, audio source and microphone selectors, a source preview, and an obvious stop button.

The audio settings screenshot suggests a focused panel for recording mode, separate game audio and
microphone tracks, source selection, volume, and a mic test. Per-app audio routing can be an advanced
option; ClipCraft's initial panel should keep the common game/system audio and mic choices visible
without reproducing every toggle shown in Medal.

Carry over the near-black charcoal canvas, white primary labels, muted gray secondary text, thin
panel separators, slim icon rail, compact capture strip, thumbnail-led library, and vivid lime
selection/action state. Keep the quick editor visually consistent with the dark shell, but simplify
it to a large preview, play/pause, one scrubber with trim handles, split/cut, and save. Do not copy
Medal's full editor tool rails or timeline depth into this product. Replace “Create Link” and cloud
actions with local “Reveal in Folder” and “Copy File Path” actions; omit cloud-sync settings.

Public reference pages (the embedded screenshots are the references):

- [Getting started with Medal on Windows](https://support.medal.tv/support/solutions/articles/48000959661-getting-started-with-medal-on-windows) — capture status, navigation, and library.
- [Medal's Advanced Video Editor](https://support.medal.tv/support/solutions/articles/48001275437-medal-s-advanced-video-editor) — editor frame, preview, tool rails, inspector, and export controls.
- [Trim, Split & Duplicate Clips](https://support.medal.tv/support/solutions/articles/48001211535-how-to-trim-split-and-duplicate-clips) — transport and timeline details.

The retrieved public screenshots used for this plan are in `/tmp/clipcraft-medal-reference/` in the
current environment, including a library view, editor, and trim timeline. The
additional library, storage, recording, and audio-settings screenshots supplied in this conversation
are also visual references. Keep all references local; do not add them to Git, bundle them, or trace
their logos or artwork. Recreate layout, palette, sizing, and interaction states with original
ClipCraft icons and generated/openly licensed media. “Less bloat” means the first app has only the
capture strip, library, compact editor, and focused local settings; no feed, profiles, friends,
quests, store, comments, accounts, links, cloud sync, or upload prompts.

The Medal support pages refer to editor “projects,” but do not document a user-facing `.ccproj`
file format. `.ccproj` was my earlier ClipCraft architecture suggestion, not a Medal format. To
keep the product clip-first, ClipCraft's primary artifacts are ordinary local video files. The app
may keep a small rebuildable local index for titles/tags/favorites, but no custom editor project or
save-project step is part of the MVP.

## MVP promise and acceptance gate

The MVP targets Windows 10/11 first because game and desktop capture coverage is strongest there.
The platform boundary must leave room for macOS ScreenCaptureKit and Linux PipeWire/portal providers
from the beginning. The initial capture target is a monitor or application window; game-specific
hooks, kernel drivers, and anti-cheat-sensitive injection are deliberately out of scope.

MVP capabilities:

1. First-run setup discovers displays/windows and lets the user choose a source, replay duration
   (15/30/60/120 seconds), output folder, video quality, microphone, and a global clip hotkey.
2. A bounded replay buffer continuously captures 1080p60 when the machine supports it. It keeps
  encoded video segments plus timestamped audio, not an unbounded raw-frame queue.
3. The hotkey writes a clip consisting of pre-roll and optional post-roll. The UI shows capture,
   buffer, disk, and encoder state and fails with a useful error when a capability is unavailable.
4. System/game audio and microphone are separate, synchronized tracks with simple mute and gain
   controls. Advanced routing and webcam overlays are outside the first release.
5. The library displays thumbnails, duration, source, capture time, and processing state. Opening a
   clip shows its player and local details. The library can rename, tag, favorite, group into local
   collections, reveal, delete, and reopen clips without an account.
6. A basic quick editor supports preview, frame-accurate start/end trim, cut/split, and saving the
   result as a new local video file. No multitrack timeline, effects, captions, overlays, or
   editor-project workflow in the MVP.
7. Export writes H.264/AAC MP4 through a hardware encoder when available and a tested software
   fallback otherwise. GIF, WebM, AV1, and advanced effects follow after the gate.
8. `clipcraft-cli` can inspect local clips, trim/split, export, and run the same command IDs as the
   desktop app. It uses local files and standard input/output; no control server is required.

The MVP is accepted when a clean install can reach a locally saved clip in under two minutes, a
hotkey press produces the configured duration without A/V drift, a 30-minute capture session stays
within configured memory/disk budgets, export succeeds on hardware and software paths, and the
exported file plays after restarting the app. Verify the complete path with network access disabled
and without starting a service. A two-hour soak, suspend/resume test, display
hot-plug test, and full-disk test are release checks rather than demo-only checks.

## Scope boundaries

ClipCraft has no hosted component in this plan. Installing, launching, capturing, managing the
library, editing, and exporting must work offline and must not depend on a hosted server or a
separately running service. The MVP also excludes livestreaming, mobile clients, browser capture,
full Premiere-style editing, kernel-level capture, DRM bypass, hidden/background capture without an
explicit user setting, cloud upload/sync, accounts, share links, comments, likes, moderation,
monetisation, and AI highlight detection. Users can share exported files with tools of their choice.

## Issue-level limitations and requirements

Every issue must state the user problem, evidence, target behavior, affected Rust crates/platforms,
unsupported cases and fallbacks, dependencies, acceptance criteria, and how each criterion will be
verified. All issues inherit the non-negotiable pure-Rust, offline/no-server, safe-input, bounded
resource, local-media, and clean-room rules in [`AGENTS.md`](AGENTS.md). Feature issues specify
default/empty/error states; bugs provide reproduction steps and regression tests; platform work
names OS/API/permission coverage; performance work includes repeatable baseline and target values.
The full definition and GitHub templates are in
[`docs/issue-requirements.md`](docs/issue-requirements.md) and `.github/ISSUE_TEMPLATE/`. No issue
is complete without evidence, relevant tests, and updates to the roadmap/gap record.

## Proposed repository shape

Keep the dependency direction explicit and make the engine portable across native platforms:

```text
L0  time, geom, color, frame, bitstream
L1  capture-model, audio, media, library
L2  cut/trim edit, codecs, thumbnails
L3  export jobs
L4  engine (Session + commands)
L5  ui-egui, platform (OS capture/audio/hardware FFI)
L6  apps/clipcraft, apps/clipcraft-cli
```

Names can change after the FilmCraft reuse spike, but the boundaries should remain:

- `clipcraft-time` owns integer ticks, rational frame rates, sample positions, and conversions at
  the API boundary. Never use floating-point seconds for edit math.
- `clipcraft-capture-model` defines `CaptureSession`, source descriptors, capabilities, encoded
  segments, audio packets, timestamps, and failure states. It has no OS dependencies.
- `clipcraft-platform` implements providers: Windows Graphics Capture/DXGI and WASAPI loopback
  first; ScreenCaptureKit/CoreAudio and PipeWire/portal later. It is the only crate allowed to
  contain platform `unsafe`, behind a safe `Result` API with a software/no-capture fallback.
- `clipcraft-media` probes/imports files and exposes a GOP-aware `MediaSource`, frame cache, audio
  decode, thumbnails, and waveform summaries. Reuse FilmCraft codecs or extract shared crates only
  when licenses, APIs, and maintenance ownership are clear.
- `clipcraft-replay` owns the bounded segment ring. Segment metadata includes first/last timestamp,
  keyframe positions, audio span, codec parameters, and a content hash. A save operation atomically
  snapshots the required segments, flushes post-roll, and records any keyframe re-encode needed at
  the requested in-point.
- `clipcraft-library` indexes normal local video files and stores small app metadata such as title,
  tags, favorite state, capture time, and thumbnail location. It can rebuild its index from the
  configured folders; missing files appear as offline entries. The video file remains the user's
  primary artifact.
- `clipcraft-edit` holds simple in-memory cut/trim ranges and produces a new local video file. It
  does not require a user-facing project document or custom `.ccproj` format.
- `clipcraft-engine` owns capture/library state, validation, local background jobs, recovery, and
  commands such as `capture.start`, `capture.clip`, `library.list`, `library.reveal`, `clip.trim`,
  `clip.split`, and `export.queue`.
- `clipcraft-library` metadata and caches live in an app data directory, are versioned and recoverable,
  and can be rebuilt from the original local video files.
- `clipcraft-ui-egui` is a view over engine state. The CLI and UI dispatch the same commands. Capture
  status is visible without opening the editor; no web viewer, backend, or server component is part
  of the product.

## Capture and media design

The capture path is the highest-risk subsystem and gets a design spike before UI work:

1. Confirm per-platform permission prompts, cursor inclusion, HDR/SDR conversion, display scaling,
   multi-monitor coordinates, occlusion behaviour, and capture of protected windows.
2. Choose a hardware encoder through a capability probe. Use short GOPs and independent segment
   files so the replay buffer can evict old data without rewriting the whole recording.
3. Timestamp video and each audio source against one monotonic clock. Resample only at the edge;
   store the original sample rate and clock correction in metadata.
4. Bound every queue by bytes, duration, and item count. Backpressure must drop or degrade preview
   frames before it drops encoded clip data, and every drop must be observable.
5. Make privacy visible: a clear armed/recording indicator, a configurable hotkey to pause capture,
   per-source mute, a local-only storage label, and a one-click purge of the replay buffer.

The first performance budget is 1080p60 capture with <250 ms hotkey-to-armed acknowledgement, no
unbounded memory growth, and a saved clip available within five seconds after post-roll ends on a
reference machine. Measure encode latency, dropped frames, audio drift, disk throughput, and power
draw rather than relying on a successful process start.

## UI flow

- **App frame:** a narrow left icon rail and a compact top strip patterned after the reference:
  detected game/source, hotkey, replay length, recording state, long-recording toggle, resolution,
  frame rate, and settings. Keep social counters, promotions, account entry points, and cloud status
  out of this frame.
- **Home/library:** a dense four-column clip grid on desktop, with thumbnail and duration badge,
  title, compact game/“On Device”/file-size/date metadata, and local “Reveal in Folder” /
  “Copy File Path”
  actions. Keep Import, New Edit, and compact filters in the toolbar. Use the dark shell and quiet
  separators from the reference; reserve lime for selected/active states and primary actions.
- **Clip details:** opening a library card shows a large local preview with a compact details panel
  for title, date, game/source, file location, tags, and local collection. Keep the layout close to
  Medal's detail view; omit likes, comments, views, social sharing, and online profile links.
- **Capture HUD:** compact always-on-top status with buffer duration, source, audio meters, and a
  clip-saved toast. It must be optional and never obscure the captured content.
- **Quick editor:** a large preview with play/pause, a simple scrubber, start/end trim handles,
  split/cut, undo, and a lime “Save Clip” action. Keep it a focused panel or simple editor window;
  no multitrack timeline, tool rails, effects browser, overlays, or project-document workflow in the
  MVP. The main product remains capture and library.
- **Capture settings:** show a clear capture toggle, source preview, system/game-audio and microphone
  selectors, and an obvious stop action. **PC Storage:** choose a local capture folder, set a local
  storage limit, and show used/free space with clips separated from other files. No cloud-sync
  section. Every permission failure explains how to fix it.
- **Audio settings:** choose the recording mode and microphone, keep game/system audio and the mic
  as separate tracks, expose their levels, and provide a simple mic test. Per-app audio routing and
  noise suppression are advanced follow-ups, not required in the first UI.
- **Other settings:** hotkeys, replay duration, quality, privacy, and diagnostics. Do not show
  upload or account controls.

## Milestones

### M0 — product and architecture spike

Write the capability matrix for Windows/macOS/Linux, benchmark capture APIs on reference hardware,
decide whether FilmCraft crates can be consumed directly, create the Cargo workspace, layering
check, lint policy, attribution policy, and minimal `clipcraft-cli`, then validate the contributor
docs and issue forms scaffolded in this repository. Exit when a synthetic frame and audio stream can
pass through the proposed timestamp API, one `cargo xtask ci` command works from a clean checkout,
and the Rust, offline, visual, and product decisions above are recorded.

### M1 — local media and library foundation

Implement `Tick`, frame/audio buffers, file probing, local library index and metadata, atomic index
saves, thumbnail cache, local playback preview, and CLI inspect/import/export. Add property tests for
time conversions, malformed media/metadata inputs, and bounded allocation behaviour. Clips remain
normal local video files; do not add `.ccproj`.

### M2 — Windows capture and replay buffer

Implement monitor/window capture, capability reporting, hardware/software encoder selection, the
bounded segment ring, hotkey lifecycle, disk quota enforcement, and a headless capture smoke test.
Use generated frames and audio for deterministic tests; use real capture only in platform integration
tests.

### M3 — audio and clip finalization

Add WASAPI loopback and microphone sources, clock alignment, simple mute/gain, post-roll, and
recovery from device changes. Verify sync over long captures, device disconnect, sleep/wake, and
full-disk conditions.

### M4 — lightweight editor and export

Add the egui clip library and quick cut/trim editor, undo for the active edit, and export jobs with
progress/cancel. The CLI and UI must use the same cut/trim/export commands. Keep advanced editing
and a GPU compositor out of scope for the first release.

### M5 — release hardening

Add crash recovery, startup diagnostics, signed installers, update-safe migrations, two-hour soak
tests, performance budgets, accessibility/keyboard navigation, and Windows permission guidance.
Document all known platform limitations honestly before calling the build beta-quality.

### M6 — additional local platforms

Add macOS and Linux capture/audio providers behind the same Rust platform interfaces. Verify each
platform's permission flow, display/window capture, audio routing, recovery, and export locally.
There is no sharing service, web client, user account system, or hosted milestone.

## Quality and security gates

- Unit/property/fuzz tests for library metadata parsing, timestamp conversion, ring eviction,
  segment joins, cut/trim algebra, audio resampling, and every command's hostile parameters.
- Golden tests for generated thumbnails and exported MP4 probe results. Use ffmpeg/ffprobe only as
  external test or fixture oracles, never as a shipped dependency.
- Integration tests for hotkey → clip, device loss, permission denial, hot-plug, suspend/resume,
  low disk, crash recovery, and repeated start/stop. Run capture tests on real platform runners.
- Security tests for path traversal, malformed media, oversized metadata, storage quotas, safe
  deletion, and log redaction. Do not log captured paths, microphone data, or raw environment values.
- Release gates include `cargo fmt --check`, workspace tests, clippy with warnings denied, dependency
  license/audit checks, layering, and a measured performance report.

## First implementation sequence

1. Keep this plan and the scaffolded contributor/issue docs as the product contract; assign stable
   IDs to the initial backlog items and refine acceptance criteria before implementation expands.
2. Create the workspace and a vertical Rust CLI slice: generated media → local library entry →
   trim/split → standard MP4.
3. Add the platform capture trait and a deterministic fake provider before any OS FFI.
4. Implement the bounded replay buffer and timestamp/clock tests.
5. Land the Windows provider and a small capture HUD; then add audio and post-roll.
6. Build the capture-first library and quick cut/trim UI around the same engine commands.
7. Extend the same local capture, audio, recovery, and export contract to macOS and Linux when the
   Windows release gate is met.

Open decisions to resolve at M0: the minimum Windows version, whether to consume FilmCraft crates or
extract shared media crates, the first hardware encoder matrix, local library metadata/index format,
storage quota defaults, and per-platform source-selection/permission behaviour. No custom project
format is required for the MVP. All decisions must preserve offline operation and the no-hosted-server
requirement.
