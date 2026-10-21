# ClipCraft plan

**Status:** active prototype; local capture/replay, playback, library, original-quality trim, and estimated-size H.264 export are implemented with platform-specific limits
**Last reviewed:** 2026-10-21
**Product:** a local-first instant-clip recorder, clip library, and basic cut tool for players

This document describes product direction and the intended MVP, not a claim that every planned
capability is complete. For the current implementation and verification limits, see
[`README.md`](README.md), [`ROADMAP.md`](ROADMAP.md), and [`docs/gaps.md`](docs/gaps.md).

**Implementation requirement: pure Rust.** First-party product code, UI, CLI, and required tooling
belong in a Rust Cargo workspace. No JavaScript/TypeScript frontend or non-Rust app runtime.
**Offline-first requirement:** capture, library, editing, settings, and export must work without a
network connection or server process. Optional, user-configured encrypted backup is a planned
network feature; it must not make ClipCraft accounts, ClipCraft-hosted storage, or an online core
workflow necessary.

## Product direction

ClipCraft should make the path from “that just happened” to a clip saved on the user's computer
take one hotkey and a few seconds. The core loop is:

```text
choose capture source → keep a bounded replay buffer → press the clip hotkey
→ save the last N seconds (+ optional post-roll) → make a quick cut or trim → export locally
```

Capture and the local clip library are the product. The first editor is a quick trim/cut tool; a
more capable but still focused one-track editor is a later proposal, not an MVP dependency. ClipCraft
is an offline-first desktop app: capture, library, editing, settings, and export work without an
account, network connection, or server. Recordings stay on the user's computer unless the user
explicitly configures encrypted backup to a storage provider. There is no community or social feed.
Low capture overhead is a product goal; the current CPU-frame pipeline does not yet match the
GPU-resident capture-to-encode path described by Vice.

[Medal.tv](https://medal.tv/) is the feature reference for the loop: instant replay, configurable
hotkeys and durations, game/window capture, system audio plus microphone, a clip library, quick
edits, and local storage controls. It is inspiration for behaviour and workflows only. ClipCraft will
recreate the compact dark capture-first look as closely as possible in layout, palette, sizing,
hierarchy, control density, library cards, editor arrangement, and lime action/selection states.
ClipCraft will use its own name, icons, labels, and visual assets, and will not use Medal's logo,
source code, network protocol, or proprietary media.

## Pure-Rust implementation and repository practice

ClipCraft is an independent Rust implementation that reuses selected, pinned open-source
dependencies, including FilmCraft media crates. It is not a formal clean-room implementation. The
desktop app, UI, capture and editing engines, CLI, local library metadata, and automation are Rust
workspace code. Use a native Rust UI (egui/eframe); do not add a JavaScript/TypeScript app, Electron
or Tauri shell, hosted backend, or bundled FFmpeg runtime. The app must not need a server process to
run. Review dependency licenses, pinned sources, API fit, and safety implications; implement file
formats from public specifications. FFmpeg/ffprobe may be used as external fixture generators or
test oracles, never linked or shipped. Platform capture and hardware APIs sit behind a small Rust
platform crate and safe `Result`-returning interfaces; isolate and document any necessary OS FFI
there, with a fallback wherever practical.

Follow the ArtCraft documentation model and tailor it to ClipCraft:

- `AGENTS.md` is the short, authoritative set of rules for all contributors: never crash or lose
  work, no panic shortcuts in non-test code, hostile-input bounds, dependency/licence rules, and
  the limited platform-FFI boundary.
- `CLAUDE.md` is the working guide: read `AGENTS.md`, then the current roadmap/gaps, architecture,
  and relevant crate docs; keep work autonomous and follow the repository's single quality gate.
- `README.md` explains the product, Rust-only stack, current status, setup, run/test commands,
  architecture links, and original ClipCraft screenshots.
- Public planning and engineering references live in `ROADMAP.md` and `docs/` (`architecture`,
  `agents`, `backlog`, `contributing`, `testing`, `gaps`, issue requirements, and short ADRs). Keep
  detailed working notes and Medal reference screenshots under a gitignored local `plan/medal/`
  directory.
- Documentation ships with implementation. Every feature change updates the README's current
  capabilities and `docs/gaps.md`'s remaining limits/evidence in the same pull request; update the
  roadmap, backlog, architecture, and testing pages when implementation changes their status or
  guidance. The PR gate checks the README and gaps report.
- Give milestones and tasks stable IDs. Use one `cargo xtask ci` entry point for formatting,
  linting, tests, dependency/layer checks, and platform checks; CI and contributor instructions call
  the same gate. Keep the engine command registry shared by the UI and CLI so features can be driven
  and tested without clicking through the app or starting a control server.

The product model and interaction patterns may be inspired by ArtCraft siblings; do not copy their
source code. Reuse a sibling crate only after checking its license, API fit, and maintenance cost.
Selected code from Vice and ClipRecorder is being translated into Rust under permission reported by
the ClipCraft maintainer; the exact ports and upstream revisions are tracked in
[`docs/upstream-ports.md`](docs/upstream-ports.md).

## Principles taken from the ArtCraft repositories

| Reference | Adopt | Adapt for ClipCraft |
| --- | --- | --- |
| [EffectCraft](https://github.com/storytold/effectcraft) | engine-first design, exact time math, render jobs, CLI automation, evidence-based issue workflow | keep the editor intentionally small; prioritize capture latency and bounded memory |
| [PhotoCraft](https://github.com/storytold/photocraft) | thin egui shell, one command registry for UI and CLI, versioned local metadata, no-panic input handling, contributor/agent guidance | keep the UI/CLI command model and no-panic rules; do not make a custom edit-project format the center of a clip recorder |
| [FilmCraft](https://github.com/storytold/filmcraft) (sibling video project) | reusable media/frame/audio abstractions, rational timebase, codecs, export jobs, platform hardware adapters | extract or depend on shared crates only after an API review; do not fork a large editor wholesale |
| Medal.tv | one-key capture, replay buffer, clip library, quick editing, local storage controls | keep the familiar capture-to-export loop; omit social, account, and link features; evaluate opt-in encrypted backup separately |
| [Vice](https://github.com/splitshotz/Vice) | Linux instant replay, game labels/playlists, Discord presence, keyboard overlay, and a built-in editor; GPL-3.0-or-later Python app using a separate GPU recorder and web UI | the longest-match game-name rule is ported into Rust for future process detection; no bundled game database, UI, or non-Rust runtime; exclude daemon-at-login and public link/tunnel behavior |
| [GPU Screen Recorder](https://git.dec05eba.com/gpu-screen-recorder/) | Linux GPU capture and hardware encoding across vendor/compositor combinations, with driver and permission caveats | implement equivalent behavior independently in Rust against public platform APIs for M5.2; do not translate/copy its GPL-3.0-only source, ship its C/C++/FFmpeg runtime, or require its privileged KMS helper |
| [ClipRecorder](https://github.com/By183/ClipRecorder) | Windows Graphics Capture → D3D11 conversion → generic Media Foundation H.264 MFT → encoded ring buffer → MP4 save, with WASAPI audio | its timestamped audio gap/overlap policy is ported as a Rust utility; keep the existing Rust capture/encoder path and do not add the C++ app or assets |
| [Segra](https://segra.tv/) | community and creator-facing product patterns | review for product research; community features are explicitly out of scope |
| [NVIDIA ShadowPlay](https://www.nvidia.com/en-us/geforce/geforce-experience/shadowplay/) | game-triggered capture, replay, and controller-oriented save-clip flow | use as a behavior reference; no NVIDIA-only SDK or vendor lock-in |
| ScreenCraft | focused creator-video editing patterns | review for small editor interactions; do not let an editor-first product reshape the capture-first MVP |

Use the public [EffectCraft `AGENTS.md`](https://github.com/storytold/effectcraft/blob/main/AGENTS.md),
[`CLAUDE.md`](https://github.com/storytold/effectcraft/blob/main/CLAUDE.md), and
[README](https://github.com/storytold/effectcraft/blob/main/README.md), alongside the corresponding
[PhotoCraft `AGENTS.md`](https://github.com/storytold/photocraft/blob/main/AGENTS.md),
[README](https://github.com/storytold/photocraft/blob/main/README.md),
[`docs/contributing.md`](https://github.com/storytold/photocraft/blob/main/docs/contributing.md),
and [`docs/architecture.md`](https://github.com/storytold/photocraft/blob/main/docs/architecture.md)
as concrete examples for contributor rules, agent workflow, issue requirements, and product/setup
documentation described below.

Use public product behavior and standards as references, never copy proprietary code or assets, and
track reused open-source crates and third-party assets with explicit source, license, and attribution
records. See [ADR 0004](docs/adr/0004-filmcraft-crates.md) for the FilmCraft dependency review.

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
actions with local “Reveal in Folder” and “Copy File Path” actions. Keep backup out of the main
capture/library surfaces; a future backup feature belongs in a separate, deliberate settings flow.

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
capture strip, library, compact editor, and focused settings; no feed, profiles, friends, quests,
store, comments, accounts, or share links. Optional encrypted backup is a later, separate utility
rather than a social or library-sharing feature.

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

MVP target capabilities (not all implemented; check [`ROADMAP.md`](ROADMAP.md) and
[`docs/gaps.md`](docs/gaps.md) for delivery and evidence status):

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

## Planned expansion after the current local workflow

These are product proposals, not implemented features or release promises. Work is staged in
[`ROADMAP.md`](ROADMAP.md) and specified as bounded issues in [`docs/backlog.md`](docs/backlog.md).
The order prioritizes the user's explicit request that capture only arms for selected games, then
platform breadth, backup safety, and optional integrations. Project comparisons are research input,
not permission to copy code, assets, or proprietary interfaces.

- **Game-aware capture:** default to Game-only mode: detect an explicitly allowlisted game and arm
  the replay buffer only while it runs. Do not capture the desktop while idle. A separate, deliberate
  one-session desktop-capture action may be available. Detection must not read process memory,
  inject code, install drivers, or bypass anti-cheat.
- **Platform breadth:** target tested Windows, Linux, and macOS desktop workflows through shared
  Rust core APIs. Android is a feasibility study only until screen-capture permission, foreground
  service, battery, thermal, storage, and game-launch constraints are proven.
- **Low-copy GPU capture:** evaluate native GPU-surface handoff from capture through conversion and
  hardware encoding, starting with the best-supported Windows and Linux paths. Do not ship
  `gpu-screen-recorder` as a C/C++/FFmpeg runtime; use its public behavior as a reference only. Avoid
  privileged KMS helpers, and keep the existing CPU path as a visible fallback. Treat Vice's
  sub-1% CPU figure as a target to measure on named hardware, not a universal promise.
- **Encrypted backup:** optionally back up user-selected clips and metadata after encrypting them on
  the device. The user chooses and configures a storage provider. No ClipCraft account or hosted
  storage is required, and core workflows remain available offline. Provider choice, key recovery,
  metadata exposure, deletion, and restore behavior must be settled before implementation.
- **Controller and presence integrations:** optional controller shortcuts and Discord Rich Presence
  can be considered after desktop platform parity. Both are off by default and must not upload clips
  or inspect game memory.
- **Themes, overlays, and plugins:** start with built-in keyboard/mouse overlays and declarative
  themes. A plugin API and shader/effect extensions require a security and portability spike first;
  never load unrestricted native plugins by default.
- **CLI and AI-agent access:** grow the existing local CLI and consider an MCP stdio adapter for
  managing and editing clips. Keep the interface local and permission-scoped; ClipCraft itself must
  not send footage to an AI provider. Destructive operations require explicit user authorization.
- **Editor growth:** after trim/cut and export are reliable across target platforms, evaluate a
  focused one-track editor with a few transitions, text/keyboard/mouse overlays, and simple audio
  controls. Do not promise a CapCut/Clipchamp replacement or make multitrack editing a core
  requirement. Video and microphone/system audio recording are already part of the current capture
  plan; voice/video calls and community features remain out of scope.

The product boundary remains: no livestreaming, web client, social feed, accounts, share links,
comments, likes, moderation, monetisation, privileged capture helpers, driver injection, DRM bypass,
or hidden capture. Explicitly configured optional backup is the only planned core workflow that may
need a network.

## Issue-level limitations and requirements

Every issue must state the user problem, evidence, target behavior, affected Rust crates/platforms,
unsupported cases and fallbacks, dependencies, acceptance criteria, and how each criterion will be
verified. All issues inherit the non-negotiable pure-Rust, offline-first/no-required-server,
safe-input, bounded-resource, local-media, and original-code/licensing rules in [`AGENTS.md`](AGENTS.md).
Optional network features must be opt-in and must not require ClipCraft-hosted services. Feature issues specify
default/empty/error states; bugs provide reproduction steps and regression tests; platform work
names OS/API/permission coverage; performance work includes repeatable baseline and target values.
The full definition and GitHub templates are in
[`docs/issue-requirements.md`](docs/issue-requirements.md) and `.github/ISSUE_TEMPLATE/`. No issue
is complete without evidence, relevant tests, and updates to the roadmap/gap record.

## Current repository architecture

The implementation uses the dependency layers and workspace crates in
[`docs/architecture.md`](docs/architecture.md); that page is the source of truth for current crate
names and boundaries. The main pieces are `clipcraft-time` and `clipcraft-frame`,
`clipcraft-capture` and `clipcraft-media`, `clipcraft-player` and `clipcraft-export`,
`clipcraft-record`, `clipcraft-engine`, `clipcraft-audio-out` and `clipcraft-platform`, plus the
desktop app, CLI, and Rust `xtask`.

The app uses FilmCraft's pinned time and media crates for decoding, encoding, container support, and
hardware encoder adapters ([ADR 0004](docs/adr/0004-filmcraft-crates.md)). ClipCraft owns the UI,
platform-provider integration, replay buffer, recording/session logic, local index, and trim flow.
The library index is local and rebuildable; normal video files remain the user's primary artifacts.
The editor is currently playback plus a basic trim-to-new-MP4 operation. There is no
user-facing `.ccproj`, multitrack timeline, general effect system, or server component.

The exact crate graph is checked with `cargo xtask layers`. Platform adapters expose fallible safe
interfaces, and platform-specific `unsafe` stays in the reviewed boundary. Capture provider evidence
and format/performance limits are recorded in [`docs/gaps.md`](docs/gaps.md).

## Capture and media constraints

The capture/replay/save pipeline is implemented. Changes to it must preserve these requirements and
add platform evidence for any new behavior; open verification is listed in [`docs/gaps.md`](docs/gaps.md):

1. Confirm per-platform permission prompts, cursor inclusion, HDR/SDR conversion, display scaling,
   multi-monitor coordinates, occlusion behaviour, and capture of protected windows.
2. Choose an encoder through a capability probe, prefer hardware where available, and report a
   software fallback. Keep GOPs short enough to bound replay start error and buffer encoded media
   within explicit duration/byte limits.
3. Timestamp video and each audio source against one monotonic clock. Resample only at the edge;
   store the original sample rate and clock correction in metadata.
4. Bound every queue by bytes, duration, and item count. Backpressure must drop or degrade preview
   frames before it drops encoded clip data, and every drop must be observable.
5. Make privacy visible: a clear armed/recording indicator, a configurable hotkey to pause capture,
   per-source mute, a local-only storage label, and a one-click purge of the replay buffer.
6. For any future low-copy path, implement the handoff independently in Rust against documented
   platform APIs. Keep compatible capture surfaces on the same GPU through conversion and hardware
   encoding, and bound their lifetime. Preserve the current CPU-frame fallback when a device,
   driver, or compositor cannot share a surface safely. Do not translate/copy GSR source under the
   current permissive-dependency policy.

The first performance budget is 1080p60 capture with <250 ms hotkey-to-armed acknowledgement, no
unbounded memory growth, and a saved clip available within five seconds after post-roll ends on a
reference machine. Measure encode latency, dropped frames, audio drift, disk throughput, and power
draw rather than relying on a successful process start. The current capture interface reads every
frame into CPU-owned BGRA memory before conversion; a low-copy GPU-resident path is planned in
M5.2. Vice reports sub-1% CPU for its own backend, but that number is not a ClipCraft requirement
or claim until it is measured on a named device, driver, compositor, resolution, and frame rate.

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
  storage limit, and show used/free space with clips separated from other files. Do not add an
  always-visible sync status; any future backup gets a separate explicit setup flow. Every
  permission failure explains how to fix it.
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
tests, performance budgets, accessibility/keyboard navigation, Windows permission guidance, and the
planned low-copy GPU capture-to-encode path after platform feasibility checks. Document all known
platform limitations honestly before calling the build beta-quality.

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

Open decisions to resolve at M0 (answers and deferrals are recorded in `docs/adr/0002` and `docs/adr/0003`): the minimum Windows version, whether to consume FilmCraft crates or
extract shared media crates, the first hardware encoder matrix, local library metadata/index format,
storage quota defaults, and per-platform source-selection/permission behaviour. No custom project
format is required for the MVP. All decisions must preserve offline core workflows and the
no-required-hosted-service rule; optional backup is governed by the M9 design ADR.
