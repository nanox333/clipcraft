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
Expect a GPU to save roughly a third of the time, and to free the CPU while it runs, not to
make export several times faster.

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
