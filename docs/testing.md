# Testing

The test strategy treats capture, media, and local library metadata as untrusted and checks failure
behavior as carefully as the happy path. Capture/replay, playback, persistent library metadata,
audio, and trim export have deterministic tests; platform evidence and real-device limits are
recorded in [`gaps.md`](gaps.md) and [`release-checklist.md`](release-checklist.md).

## Canonical gate

Run `cargo xtask ci` before completing an issue. CI calls the same command. It includes formatting,
strict Clippy, workspace tests, layer validation, asset attribution checks, and a `cargo-deny`
license audit. Keep focused test commands in crate docs, but do not create a second gate with
different pass criteria.

Current focused checks include `cargo test -p clipcraft-time` for time math, `cargo test -p clipcraft-media` for container probing
including byte-corruption and oversized-header cases, `cargo test -p clipcraft-player` for playback, sound, speed, stepping, looping, track selection, stalled devices, and thumbnails (it encodes its own H.264 clips in pure Rust, so it needs no
external tools), `cargo test -p clipcraft-frame` for queue limits,
`cargo test -p clipcraft-engine` for bounded local scans and command dispatch, and
`cargo test -p clipcraft` for UI helpers, keyboard handling, and smoke tests that draw every screen
(library, empty states, details, settings, the player windowed and fullscreen) at window sizes from
1×1 to 6000×4000 with awkward titles, so a layout that panics fails the gate.

Long-run and release evidence is described in [`release-checklist.md`](release-checklist.md):
`cargo run --release -p clipcraft-record --example soak -- --minutes N [--freeze-for S]` measures
memory, processor time, dropped pictures, and clip validity over time, and can freeze the recorder
to simulate sleep.

## Documentation gate

Documentation ships in the same pull request as implementation. For any changes under `apps/`,
`crates/`, or `tools/` (or workspace dependency manifests), update `README.md` and `docs/gaps.md`;
update the matching backlog and roadmap when a status changes, and update architecture/testing/CLI/
release/ADR guidance as needed. Pull-request CI runs `cargo xtask docs-check origin/<base-branch>`
and fails when an implementation diff does not include both `README.md` and `docs/gaps.md`. The
same check runs on direct pushes to `main`, against the previous commit.
Both workflow steps explicitly use Bash on Linux and Windows so `$BASE_REF` expands consistently.

To run the same check locally, first fetch the target branch and pass its remote-tracking name:

```sh
git fetch origin main
cargo xtask docs-check origin/main
```

The check uses the merge base with the supplied ref and includes committed, staged, unstaged, and
untracked local changes. Documentation-only changes pass without requiring README or gap-report
edits; implementation changes require both.

## Required evidence by area

### M7.1b — Background discovery and desktop/game switching (2026-10-22)

Engine fixtures cover bounded VDF/ACF parsing, alternate Steam libraries, invalid IDs/paths,
corrupt and oversized metadata, cancellation, custom-first matching, exclusions and safe settings
migration. One blocked discovery worker accepts 50 replacement requests while retaining just one
pending request. Failure, panic and obsolete-settings results cannot be mistaken for game exits
or start capture. A Linux regression test launches the Rust test binary as a child, kills it and
leaves it unreaped during observation: its zombie entry must no longer count as a running game.

Generated capture providers exercise game launch → save → desktop → reopened game → second exit
→ fresh timer → expiry. Four valid MP4s result, with two captured-game labels and two untagged
desktop clips; an existing sentinel file stays unchanged. Additional checks cover continuous
startup, manual Stop/Start, rule removal, failed game opens, bounded saving and a game that exits
while both save slots are occupied. The last case retains and saves the desktop buffer rather
than discarding it when the pending destination disappears. UI tests activate the three recording
modes and duration controls with Tab/Enter, preserve durations across modes, and exercise existing
layout extremes. `cargo xtask ci` and the branch documentation gate pass on Linux.

The actual desktop build ran on private Xvfb display `:84` at 1440×900. A second Rust app binary,
renamed as a generated game, supplied a real PID and X11 window; a small test-only Rust helper set
`_NET_CLIENT_LIST`, which a window manager normally supplies. The production process observer,
window provider, software encoder, desktop provider, save jobs and library metadata handled:

- Game exit → desktop countdown → reopening within 15 seconds → game clipping → second exit →
  fresh 15-second countdown → idle at expiry. Four replay outputs decoded without errors using
  external FFmpeg/ffprobe; metadata contained two game labels and two absent desktop game labels.
- An unreaped generated child exiting → desktop continuation and expiry.
- Continuous desktop → generated game → desktop, followed by explicit Stop staying paused across
  subsequent process scans.

Screenshots stay outside Git under `.clipcraft-local/game-switching/`: `before.png`, `after.png`,
`game-running.png`, `desktop-countdown.png`, `reopened.png`, `second-countdown.png`,
`timer-ended.png`, and `always-{desktop,game,return,paused}.png`. The final settings view wraps help
text without overlap. Six X11 provider integration tests also pass with Xvfb explicitly on PATH,
including exact PID selection, monitor/window capture, window loss and global hotkeys.

These are generated-window Linux checks, not real-game, Windows, GPU-performance or audio-device
results. X11 can return black frames for an obscured window in this virtual display; valid files
alone do not prove visual capture quality for games. Wayland process-to-window mapping and macOS
capture remain unsupported. No reference footage, test helper or external oracle ships in the app.

### M4.1b — Recursive library and captured-game labels (2026-10-22)

Built on performance branch revision `4dd91a7`. Engine/desktop tests cover nested game folders,
stable ordering, whole-tree entry/result budgets, eight-level depth enforcement, skipped symlink
cycles/aliases, non-UTF-8 paths, cancellation, duplicate scan payloads, backward-compatible index
loading, metadata edits during traversal, and captured-game/manual-label priority across rescans
and restarts. The unreadable-child test exercised actual Unix permission denial as the unprivileged
Linux user, rather than relying on a privileged run. A generated recording verifies the SaveJob's
game snapshot even when the original game string changes. Completed-save UI tests verify game
counts, metadata-write failure warnings, preserved media, and the in-memory label fallback.

The desktop worker replacement test submits 50 scans while retaining one worker thread. An egui
keyboard/accessibility test traverses the real library controls with Tab, reaches the named
**Cancel scan** button, activates it with Enter, and checks that cancellation leaves the old library
intact. Existing UI layout/accessibility and library persistence tests remain enabled.

Native UI evidence uses Xvfb at 1440×900 and four generated H.264 test-pattern clips: one in the
root, two below `Hades/` (including a second-level session folder), and one below `Celeste/`.
The previous build shows one clip; the rebuilt desktop and CLI find four. The Hades filter shows
two. The root clip has a seeded `recorded_game` label solely to inspect its UI presentation;
actual label assignment is exercised by the generated recording/completed-save tests above, not
by a real game in this screenshot. Before/after/filter screenshots are kept outside Git at
`.clipcraft-local/recursive-library/{before,after,filter}.png`.

Run `cargo xtask ci` and `cargo xtask docs-check origin/codex/m2-p1-performance` for this branch.
The full Linux gate passes. No Windows capture, game process or new Windows hardware result is
claimed; filesystem traversal cannot interrupt an OS call already blocked on an unavailable drive.

### M7.1a — Automatic known-game recognition (2026-10-22)

The engine checks every catalog executable/Steam ID, case handling and launcher lookalikes.
Default settings recognize known games with no custom registration. Custom names take priority,
even over bundled Steam IDs; per-game exclusions and the global-off switch persist and leave
custom rules intact. Tests cover start/stop arming from a recognized process fixture, no-rule
idle, bounded/normalized exclusions, unknown/oversized requests, old settings defaults and reloads.
The Games page participates in layout/accessibility checks; Tab/Enter can exclude CS2.

Native Xvfb evidence at 1440×900 shows the automatic-recognition switch, searchable known-game
checkboxes and custom override section. Before/after evidence remains ignored at
`.clipcraft-local/game-catalog/`. Isolated CLI checks cover catalog listing, enable/disable,
unknown-ID atomicity, bulk changes, and global on/off without modifying custom rules. The full
Linux quality and documentation gates pass. No installed-game, Windows, anti-cheat, focused-window
or new per-platform capture result is implied by these matching/UI/settings checks.

### M7.1a — Full Vice catalog expansion (2026-10-22)

All 347 title strings from Vice `7e4b2a3` were compared with the rebuilt CLI's catalog;
none are missing. The resulting list contains 348 entries including Bedrock, 672 process aliases
and 311 Steam IDs. The original 20 stable IDs and their existing identification variants remain.
Seven launcher/generic aliases are omitted as documented in game-catalog.md. No data from
leinstay/steamdb is imported; its schema/license assessment is recorded in upstream-ports.md.

The Linux `cargo xtask ci` gate passes with every bundled process/Steam identifier checked,
case and lookalike handling, similar-title separation (Hades/Hades II), exact Steam-ID boundaries,
custom overrides, global/per-game switches and malformed/oversized changes. A regression test
excludes all 348 entries, normalizes and saves/reloads settings under the existing 64 KiB limit,
checks every exclusion, and restores the list while preserving custom rules. The UI Tab/Enter
checkbox test passes with the larger list. Isolated CLI checks also exercise `disable all`, an
explicit 348-ID request, reload, unknown-ID atomicity, partial re-enabling and preserving global-off.

Native 1440×900 Xvfb screenshots before/after expansion and a Hades search are ignored at
`.clipcraft-local/game-catalog/vice-{before,all,search}.png`. They establish layout, the dynamic
348-game count and search presentation, not installed-game identification or capture compatibility.
The documentation gate and `git diff --check` pass. Windows/live-game and reference-machine idle
performance remain unverified; catalog metadata is not a hardware-support claim.

### M4.1c — Category-local advanced settings and custom-game form (2026-10-22)

The persisted General switch now discloses extra controls at the bottom of Games, Capture,
Quality and Audio; there is no Advanced navigation category. Regression coverage verifies each
category's controls and **Advanced settings** heading appear only when enabled. Tab/Enter toggles
the General switch, adds an exact custom rule, hides its editor again and verifies the rule still
matches. Selecting a non-Steam process after a Steam process clears the stale app ID while keeping
a manually entered name. Existing all-screen layout/accessibility tests now include saved custom
rules as well as the add form, covering sizes from 1×1 to 6000×4000. These checks verify safe layout
and accessible names; they are not a claim that sub-minimum native window sizes are supported.

Native Xvfb inspection at 1440×900 verifies the switch is last in General, the basic Games page
omits custom editing, and Games/Capture/Quality/Audio each have a divider and heading before their
extra controls. Before/after screenshots, including the final flat form (`games-flat.png`), existing-rule sections
and error states are ignored under
`.clipcraft-local/advanced-settings/`. A 960×900 native screenshot checks the supported narrow
window; the form uses separate padded fields and spacing in both sizes. Native keyboard interaction
corrects an invalid Steam ID, sets exact process matching and submits the form. Inspecting the
isolated saved preferences confirms the rule survives hiding its advanced editor.

`cargo xtask ci`, `cargo xtask docs-check origin/codex/m2-p1-performance`, and `git diff --check`
pass. This is settings disclosure/layout work; no new capture, Windows, game or audio-hardware
compatibility evidence is implied. The existing version-1 settings and shared save command remain.

### Other areas

The offline CPU benchmark is `cargo run --release -p clipcraft-record --example performance`.
It measures patterned-frame conversion, a 60-second encoded-video snapshot and long-GOP replay
insertion. See [performance.md](performance.md) for the machine, method, measured baseline and
remaining real-device checks. Timing is informational and is not a flaky CI assertion.

- **Time/library metadata:** property tests for rational conversions and trim algebra; round-trip,
  migration, truncation, oversized values, malformed metadata, missing media, and atomic-write tests.
- **Replay/capture:** deterministic fake-provider tests for ring eviction, limits, timestamps,
  post-roll, dropped frames, save races, start/stop, device loss, and full disk. Real capture tests
  run on platform hardware and state their OS/device matrix.
- **Low-copy GPU capture (Windows SDR partial):** test bounded surface leases, device loss, format/color
  conversion, and parity with the CPU-frame fallback. A fake native surface and hardware encoder
  already verify that compatible surfaces bypass readback and unsupported encoders use the fallback;
  Windows WGC/D3D11 and AMD Media Foundation also have generated-window evidence and an explicit
  real-device lease/readback/resize test. Reproduction and the checkpointed Windows soak example
  are in [windows-performance.md](windows-performance.md). Other GPU/format pairs remain unrun.
  Real-device evidence names the GPU,
  driver, OS, compositor, capture/encoder APIs, resolution/FPS, CPU use, GPU use, dropped frames, and
  game frame-time delta. Never infer sub-1% CPU use or broad compositor support from a compile or
  mock.
- **Game-only capture:** engine tests cover process match priority, exact Steam IDs, duplicate
  observations, process exit/restart, and manual pause. The X11 adapter uses a synthetic window
  with a published `_NET_WM_PID`; Windows still needs a real-device run, and Wayland app-to-window
  matching is unsupported. Verify that empty allowlists never scan or start capture and that the
  process table is polled only while Game-only mode is enabled.
- **Wayland:** a stand-in desktop on a private D-Bus session bus answers the ScreenCast portal;
  a real PipeWire server carries a real video node (GStreamer test source); format pods are
  compared with PipeWire's own builder; C structure layouts are asserted against the headers.
  Tests skip, saying why, without those tools. Tests against a real portal backend are
  `#[ignore]`d. See [wayland.md](wayland.md).
- **Audio:** generated clocked sources verify sync drift, mute/gain, resampling, microphone loss,
  and recovery over long runs.
- **Export:** `cargo test -p clipcraft-export` includes Rust-generated H.264/AAC MP4/MKV fixtures
  for original-quality trim and estimated-size compression. Compression tests decode the output,
  check first-picture content, 60-to-30 fps conversion, duration, separate audio tracks and click
  alignment, actual-size reporting, progress, malformed inputs, cancellation, temporary-file
  collisions, and a destination created while encoding. FFmpeg additionally checks full-file
  decoding when installed, solely as an external test oracle. Native preview screenshots and
  keyboard checks exercise the selector, custom size, and completion message. Software throughput
  measurements on 30/60-second clips are recorded separately from real-GPU capture evidence.
- **UI:** test commands/state transitions independently of widgets. For visible changes, run the
  desktop build, inspect keyboard navigation at a representative display scale, and attach a
  before/after screenshot using generated/openly licensed media.
- **CLI:** test every command's schema, invalid parameters, local path behavior, and JSON output.
  CLI tests must not start a network or control server.
- **Privacy/safety:** assert that core workflows work offline, no data leaves the device by default,
  logs are redacted, memory/disk use is bounded, deletion is safe, and source media is preserved.
- **Encrypted backup (when implemented):** use a fake provider to assert opt-in behavior, ciphertext-
  only transfer, integrity checks, retry/cancel, fresh-install restore, and documented key-loss
  behavior; test a real provider only with disposable credentials and synthetic clips.

Use synthetic fixtures in tests. Large real-media corpora are pinned by commit and SHA-256 and
fetched into ignored cache directories; never commit captured gameplay or private user media.

## GIF and loudness (M4.4, 2026-10-22)

Run `cargo test -p clipcraft-export --test gif_loudness` (GIF size/frames/loop, limits, cancel and
collision; normalise reaching the target within 1 LU, click timing within 50 ms, no clipping, FFmpeg
decode). Speed is a measurement, not a check:

```sh
cargo test -p clipcraft-export --release --test timing -- --ignored --nocapture
# CLIPCRAFT_TIMING_SECONDS=60 CLIPCRAFT_TIMING_SIZE=1920x1080@30 for other clips
```

Measured on a 4-core Linux container (not a gaming PC), release build, a 30 s 1920×1080@60
H.264/AAC clip (40 MB, FFmpeg test pattern) through the CLI:

| Export | Before (s) | After (s) |
| --- | ---: | ---: |
| Small file (~10 MB, wrote 9.0 MB, 960×540) | 142 | 26 |
| Medium file (~25 MB, wrote 22.5 MB, 1280×720) | 139 | 32 |
| GIF, first 10 s | 6.2 | 6.6 |
| Even out loudness (30 s) | — | 2.0 |

Small-file export got about 4–5× faster by (1) scaling the decoded Y'CbCr planes directly with an
integer bilinear filter instead of converting to RGBA, resampling with a general filter and
converting back, (2) preparing pictures on two threads ahead of the encoder, (3) giving the
encoder every core but one, (4) turning off adaptive quantisation and scene-cut search (small
gain), (5) interpolating each source row once for the one-channel scaler, and (6) choosing 540p
instead of 720p when the budget is under 2.5 Mbps, which also looks better at that bitrate (40.7 dB
PSNR against an FFmpeg bilinear reference). Splitting the output into runs encoded on separate
cores was tried and dropped: the machine was already CPU-bound (about 3.2 of 4 cores busy), so it
saved nothing and made the size target less exact. The remaining time is the pure-Rust H.264
encoder (about half), decoding (about a fifth) and scaling; a large further gain needs a faster
encoder or hardware encoding, not more threads.

Hardware encoding: small-file export now tries the same GPU encoder as recording
(`recording.video_encoder` = GPU; NVENC, Media Foundation or VA-API) and falls back to the software
encoder on any trouble: it will not open, fails part-way, returns a different number of pictures,
reorders, or cannot describe its stream. `cargo test -p clipcraft-export --test hardware` covers
this with a stand-in encoder behind the same seam (decoded length, size, first and later picture,
sound timing, FFmpeg decode, each kind of failure, cancel). **No real GPU encoder has been run for
export** (this container has none), so its speed and quality are unmeasured. As an upper bound the
timing test also runs the hardware path with a free encoder: 20 s for the 30 s 1080p60 clip, against
27–30 s with the software encoder, because decoding the 1080p60 source with the pure-Rust decoder
is then the limit (about 9 ms per source picture, and the decoder serialises concurrent requests).
Expect a GPU encoder alone to save roughly a third of the time, and to free the CPU while it runs.

The decoder limit has a second lever that was already in FilmCraft but unused: its platform crate
registers operating-system **hardware decoders** (NVDEC and VA-API on Linux, Media Foundation on
Windows, VideoToolbox on macOS) in front of the software one, with a built-in fallback if a stream
is refused or the decoder fails part-way. ClipCraft now registers them at start-up, switched off
(`clipcraft_platform::register_hardware_decoding`), and a small-file export that may use the GPU
switches them on only while it opens its own decoder, so playback and thumbnails behave exactly as
before. The report says `hardware_decode` and the trim panel says "GPU decoding". Here (no GPU)
the factories decline, the export is unchanged and reports `hardware_decode: false`; **real
hardware decoding is untested**, so no speed-up is claimed for it.

Output decodes cleanly. It is still about real time for 1080p, and a harder 720p noise
clip (590 MB per 30 s) ran slower than real time before and after, so expect long waits on
very noisy recordings. GIF did not get faster because its time is spent decoding and
quantising colours; it now uses several threads but the 1080p60 source dominates.

Manual check in the desktop app (Linux, virtual 1440×900 display, FFmpeg-made 1280×720 H.264/AAC
clips): Trim → Animated GIF wrote a 480×270, 192-frame GIF and offered Show in folder.
`clipcraft-cli clip normalize` on an 8 s clip reported -21.8 LUFS → -16.0 LUFS, and FFmpeg's
`ebur128` independently measured the output at -16.0 LUFS and decoded it cleanly. A 40 s GIF range
was refused with its message.

## Software export sample measurements

On 2026-10-21, the optimized dependency/debug app build exported generated `testsrc2` H.264/AAC
MP4 inputs at 320×180/60 fps in this shared cloud container. Output was 320×180/30 fps with copied
AAC. Each job ran once; wall time includes opening, decoding, encoding, and file publication.
The source files were 7.28 MB (30 s) and 14.56 MB (60 s). These low-resolution synthetic samples
verify offline behavior and target response; they do not establish gaming-PC or capture performance.

| Input duration | Target | Actual output | Wall time |
| --- | --- | --- | --- |
| 30 s | 10 MB | 7.26 MB | 3.81 s |
| 30 s | 25 MB | 7.26 MB | 2.88 s |
| 30 s | Custom 5 MB | 4.46 MB | 2.87 s |
| 60 s | 10 MB | 8.98 MB | 5.74 s |
| 60 s | 25 MB | 14.53 MB | 10.20 s |
| 60 s | Custom 5 MB | 4.48 MB | 9.90 s |

The larger targets do not force a small source to fill the budget. A separate generated full-HD
test exercises the actual 1920×1080-to-1280×720 resize and decoding path. Reproduce timing with
`time -p clipcraft-cli clip export INPUT OUTPUT 0 DURATION TARGET_MB`; FFmpeg may generate the
input fixture but is never called by the export command. Outputs, timing reports, and UI
before/after captures belong in ignored local artifact folders.

The native app was also opened at 1440×900 against a virtual X display. Exporting an 8-second
generated clip through Trim → Custom size → 1 MB wrote 844,672 bytes from a 2,913,501-byte source;
the source SHA-256 stayed unchanged and the completion message displayed the actual size and
target. T opens trim, I/O mark boundaries, text entry changes the custom target, and playback
shortcuts are suppressed while the export popup is open. A regression test covers that popup
conflict, and UI smoke tests draw all size choices at narrow and desktop widths.

## Small sequence editor evidence (2026-10-22)

`cargo xtask ci` covers exact FilmCraft-adapted splits, trim/reorder/delete/history, hostile ranges
and identifiers, media import bounds, and failed-edit immutability. Generated Rust fixtures cover
ordered MP4/MKV cuts, portrait fitting, silent sources, gain/mute, ten fractional-duration cuts
without accumulated AAC delay, cancellation/publication collisions, source preservation and a
40-segment export. The UI harness checks separate Editor navigation/media references, context
menus on a second-row thumbnail, double-click/Enter append, Tab/Enter activation, rapid command-key
undo/redo, split/delete and narrow/extreme layouts.

The native Linux app was opened on an isolated Xvfb display at 1440×900 and 1080×760 using three
generated 640×360 clips. Checks exercised Send to editor, the separate rail destination,
Details/Add to timeline menus, source references without per-card Add buttons, cuts, reorder,
volume/mute, title editing, keyboard actions and MP4 export. The shared charcoal/lime palette,
Inter Regular/Medium/SemiBold, thumbnail truncation, unstretched timeline thumbnails, focus states
and lower controls were inspected. Before/after and context/empty/narrow screenshots are kept
under ignored `.clipcraft-local/editor/`; generated media stays outside Git.

An app export produced a 12-second, 640×360/30 fps H.264 MP4 with 48 kHz stereo AAC (6,256,881 bytes).
FFprobe inspected its streams and FFmpeg decoded the full file solely as external test oracles.
All three original SHA-256 hashes were unchanged. A separate local CLI smoke check imported and
deduplicated references, rejected a batch with a missing source, applied split/delete/undo/redo,
reordered/muted, exported and decoded 360 pictures/12 seconds, and refused an existing destination.

These are software/virtual-display checks. Native file-picker integration on installed desktop
portal backends, physical audio output, Windows/macOS UI behavior, reference-machine 4K playback
and real capture hardware remain unverified here. The existing platform evidence/limitations
still apply; this editor work does not establish additional capture support or performance.

## Editor consistency and preset export evidence (M11.1b, 2026-10-22)

The follow-up to `e9b2df7` corrects the default GIF clock and editor preview/export differences,
then adds editor size presets. `cargo xtask ci` passes with the shared engine/CLI/UI paths.

- GIF regressions decode the written files and sum centisecond delays at 12/13 fps and fractional
  durations; clock unit cases cover every accepted 5–20 fps value. The CLI smoke check measured
  a 1-second/default-12-fps output at 1.00 s (previously 0.96 s), and 1.03 s at 13 fps.
- Shared raster tests check FilmCraft Inter SemiBold pixels, alpha composition, long-text and
  hostile-canvas bounds. Preview tests exercise texture reuse across placement/resize, cache
  release, and the same audio envelope at the start/quarter/middle of a fade while preserving
  stored volume. Existing generated sequence tests decode text, effects and fades.
- Preset fixtures cover Standard, 10 MB, 25 MB and custom targets, canvas tiers and impossible
  budgets. A generated 720p sequence exports 210 pictures at 852×480 for a 1 MB target. A real
  Rust encoder behind the GPU seam proves success, refusal, mid-stream loss, missing packets,
  missing stream description and malformed-packet fallback; cancellation/collision cases keep
  sources and existing destinations unchanged and progress remains monotonic.
- An isolated CLI smoke run exported two seconds from two 640×360 H.264/AAC inputs, with video
  fades, a colour effect and a fading text layer. Standard/10 MB/25 MB outputs were 1,009,343
  bytes; the 1 MB custom output was 810,861 bytes. All had 60 pictures, one 48 kHz stereo AAC
  track and valid full-file FFmpeg decode. Flat JSON clip fields were preserved alongside new
  target/settings metadata. Reusing an output, a zero custom target and a 30-minute/1 MB
  impossible audio budget were refused; source SHA-256 hashes stayed unchanged.

Native before/after evidence uses a clean parent worktree at `e9b2df7` and the changed app on an
isolated Linux/Xvfb 1440×900 display. Screenshots under ignored `.clipcraft-local/editor-presets/`
show the text preview and centered export window with its preset menu/custom target, including
1080×760 sizing. Mouse/keyboard entry set the custom target to 1 MB; Escape closed the window
without exporting and reopening retained the choice. A native 15-second/450-picture export with
a text layer wrote 862,561 bytes at 640×360/30 fps with 48 kHz stereo AAC; FFprobe inspected it
and FFmpeg fully decoded it. Cancelling another native job removed only its partial file and kept
the completed output. The three original hashes stayed unchanged. The native check uses generated
media only. Test fixtures, CLI reports and source hashes are outside Git.

The environment has no physical sound output or GPU export device. These checks establish the
software path and hardware-failure contract, not real-device sound, GPU throughput, Windows/macOS
UI behavior or parity on reference gaming hardware. GIF/codec/export oracles remain external
verification tools, never shipped product dependencies.

## Timeline analysis, playback preparation and recovery (M11.1c, 2026-10-22)

The final `cargo xtask ci` passes 520 tests across 30 targets, strict clippy, layering, assets and license
checks. The documentation gate passes. The acceptance evidence includes:

- Eleven focused player timeline tests use generated Rust media. Distinct source pictures resolve at
  exact Tick bin centres before and after trim; first-track audio and a trimmed impulse produce
  real min/max envelopes. Media without audio has `has_audio=false` and zero buckets. Tests cover
  cancellation between bounded audio reads and inside packet processing, missing/hostile media,
  range/payload limits and packet guards. A 30-minute fixture uses generated 8 Hz native PCM to
  exercise full-range timing and streaming without retaining the decoded range. It is not a
  realistic audio-throughput benchmark. Preflight regressions cover expanding/aggregate sample
  counts, metadata bytes, 32/64-bit box headers, depth/count bounds and explicit/default oversized
  encoded sizes. A relabeled ProRes source with a `u32::MAX` first-sample size is refused before
  metadata probing; both 24/48-byte ALAC cookie layouts reject oversized frame declarations before
  decoder construction.
- Cache tests cover progressive texture uploads, stale/cancelled-result rejection, byte/entry/
  queue limits, failed-key retention and source fingerprint invalidation. Playback tests exercise
  prepared-frame promotion, paused preloading without audio output, same-source target seeks,
  stale invalidation and the two-player budget including retiring workers. A slow decoder can
  delay preview rather than grow the worker count.
- Engine recovery tests round-trip cuts, text, title, media references, preset and clamped playhead
  with fresh history. Missing sources, malformed state/history, size/version limits, atomic
  preservation, temporary collisions, fixed-basename/path/symlink refusal and repeated discard are
  covered. Worker tests prove save/discard ordering, bounded shutdown and failure recovery.
- A CLI analysis of a three-second generated AAC clip produced six distinct pictures at exact
  sample bins and 64 waveform buckets, with maximum signed amplitude 4104. Source hashes were
  unchanged. CLI recovery smoke checks round-tripped a valid draft, preserved corrupt recovery on
  a rejected save and accepted repeated explicit discard. CLI commands use the same engine
  timeline and recovery implementation.

The native Linux app was opened on an isolated Xvfb display at 1440×900 with generated sources.
Before/after screenshots under ignored `.clipcraft-local/timeline-recovery/` show actual distinct
source pictures and decoded waveforms. The latest guarded app build was opened again; Fit, zoom
and horizontal panning verified source-time alignment, recorded in `zoomed-panned.png`. The
latest `after.png` shows four cuts, a text layer, decoded waveforms and the restored notice.
Keyboard checks exercised S split, Ctrl+Z undo,
Ctrl+Shift+Z redo, Right frame-step and Space play/pause. After restart, recovery restored four
cuts, one text layer, title, the 10 MB preset and the saved playhead, stopped with
empty undo/redo. A missing file retained its placeholder and edits, showed unavailable timeline
media and an actionable preview message. Generated media, decoded output and source hashes stay
outside Git.

Native corrupt recovery stayed byte-identical while the wrapped error and Discard action were
visible at 1440×900. Explicit discard removed only that metadata. Discarding a valid record kept
the current four cuts usable; Right navigation did not recreate the record, and the next launch
opened the library without recovery. Normal exit 100 ms after Right, before the 500 ms debounce,
persisted the latest frame change from 10.333333 to 10.366667 seconds (one 1/30-second frame).
All three source SHA-256 hashes
remained unchanged. The final recovery-notice/readability patch is rebuilt; six focused recovery
UI tests and strict clippy pass.

Independent review identified two additional shutdown-race cases. Their deterministic regressions
now pass within 15 focused tests (nine workspace recovery cases and six worker cases); a subsequent
read-only review passes. Final media preflight/allocation guards also cap whole-source MP4 box
metadata and sample declarations before either source open, plus audio configuration before
decoder construction. The latest guarded CLI analysis still returns six distinct pictures and
64 waveform buckets with nonzero peaks from the generated AAC source. The final application/CLI builds, full
gate and documentation check pass. M11.1c is software-complete for this slice; its native evidence
is Linux/Xvfb only.

The environment's virtual display and generated media can establish this software workflow.
Physical audio, GPU decode/export, real games, Windows/macOS UI behavior and reference-machine
latency/memory measurements remain separate evidence gates. Thumbnail/envelope payload and UI
cache budgets are not total process-memory measurements; FilmCraft decoder internals allocate
additional buffers. Recovery's 500 ms debounce and two-second exit-flush limit leave a bounded
waiting policy, not a guarantee that an edit just before a crash or filesystem stall is saved.


## Editor export quality (M11.1d, 2026-10-23)

Generated Rust integration tests export actual 1080p/60, 4K/30, portrait and ultrawide canvases,
including text, and decode them with the pinned reader and optional external FFmpeg oracle.
A 60 FPS fractional-cut sequence emits fifty pictures, keeps adjacent source motion distinct,
shortens its final partial video sample and preserves the expected audio click within 256 samples.
Software and hardware stand-ins receive the same FPS/two-second keyframe interval; missing
pictures/device loss restart safely in software. Existing preset, cancellation, no-overwrite
and original-media checks remain in the full gate. Plan tests cover 4K bounds, native aspect,
no upscale, legacy even rounding, frame-rate-aware size reduction, bitrate-only reduction and
invalid rates/sizes. Engine JSON and recovery tests check old defaults, supplied quality,
invalid values preserving existing files and restoration of quality choices. UI tests check
modal focus and block editing shortcuts while the export window is open.

The actual native Linux app ran at 1440×900 using isolated generated 1080p/60 and portrait/60
fixtures. A restart recovered 1080p/60 settings. Exporting its two-second clip with text produced
4,550,579 bytes at 1920×1080, 60/1 FPS, 120 pictures and continuous stereo AAC. External FFprobe
inspected the output and FFmpeg fully decoded it. Original source hashes stayed unchanged.
Before/after screenshots stay outside Git under `.clipcraft-local/export-quality/`.

The final `cargo xtask ci` passes **527 tests across 31 targets**, formatting, strict Clippy,
layering, asset and dependency-license checks. The documentation gate also passes. Native Tab
focus is constrained to the export window/popups and Escape dismisses it without editing the
sequence. The 960×640 window shows its controls and size warning. A 1 MB target visibly plans
960×540 at 60 FPS; the same CLI export produces 821,797 bytes, 120 pictures and a truthful
`resolution_limited` flag, and fully decodes. Source mode shows 720×1280 at 60 FPS for the portrait
fixture, including correctly framed preview text. Quality choices survive native restart. All
original SHA-256 hashes remain unchanged, with no abandoned partial output. Screenshots include
`after.png`, `size-target-reduced.png`, `dialog-minimum.png` and `portrait-source.png`.
This is software/Linux virtual-display evidence; physical GPU performance, Windows/macOS UI,
physical audio and reference-machine throughput remain unverified by this change. Quick trim
and its existing 720p/30 size presets are unchanged.
