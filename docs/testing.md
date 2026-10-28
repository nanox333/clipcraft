# Testing

## Shortcuts, first launch and installation (M5.1a)

Engine hotkey tests cover old string-only JSON, five bindings, round-trip persistence, canonical
duplicates, invalid/oversized lists and native ID resolution. A desktop fake-event regression
holds both PTT alternatives, releases each in turn and checks assignment suppresses actions.
Keyboard tests add, assign and remove a shortcut. Onboarding tests check legacy preferences skip
setup, fresh defaults request it, invalid destinations fail, and Finish persists completion and
choices through the same settings command.

Installation tests package generated files, install/upgrade/remove them, retain an unknown clip,
reject an unrelated nonempty folder and malformed ownership records, and roll back oversized or
truncated gzip payloads without changing the previous install. A Windows shell-link test uses a
temporary path with spaces/Unicode and validates its native link header without touching Start
menu or registry. Native screenshots, packaged-binary smoke evidence and final CI results are
recorded after verification below. Clean second-machine/signing checks remain open.

Windows 11 local verification on 2026-10-27: full `cargo xtask ci` passes, including the keyboard
add/assign/remove test, PTT overlap regression, corrupt/oversized installation rollback and the
native Unicode shell-link test. Logs: ignored `target/setup-ci-final.log` and
`target/setup-keyboard.log`. The first sandbox-only broad run had four existing recovery/game
configuration failures; none recur with native access in either full gate run.

`cargo xtask package` produces the portable zip and offline setup. A real setup executable installed
into an isolated workspace folder with spaces/Japanese characters, matched the application hash,
ran its installed CLI, upgraded while retaining the first executable, then removed both program
versions while preserving an unknown generated clip and an external preferences sentinel. This
uses the explicit no-integration mode; it does not claim a real Windows Apps & Features round trip.
Evidence is in ignored `target/setup-installer-evidence/install-smoke-*/result.json`.
Native installer review at 700×560 confirms license gating, Tab/Shift-Tab focus, Enter toggling
the license and Tab reaching the enabled Install action. It was closed before installation.
Screenshot: `target/setup-installer-evidence/installer.png`.

## Processed microphone listening and stereo (M3.1b, 2026-10-27)

The initial mono suppression prototype is superseded by independent stereo state and optional
mono folding. A regression compares opposite left/right inputs against independent mono models
to detect downmix/crosstalk; explicit mono instead cancels that opposite signal. Chunk/rate and
device-loss tests now account for interleaved stereo samples. The shared `PcmMonitor` test compares
AAC bytes against the ordinary recording path for stereo/mono, suppression on/off and 0/50/100%
volume. Invalid formats and oversized input must fail without publishing samples.

Native monitor tests use a fake worker for bounded replacement settings and actionable device
failure completion. Queue tests prove 200 ms eviction, stereo/surround placement, underrun and
immediate stop silence; a held worker lock cannot block the callback. Keyboard tests reach Mic
Test without opening a device and exercise mono alongside suppression. CLI rejects invalid test
durations before opening devices. The ignored
`real_processed_microphone_test_plays_and_applies_live_controls` test opens default input/output,
counts real callback frames, checks live zero gain and mono, and stops without saving sound.

Mic Test meters processed PCM and bypasses recording/PTT gates. User listening is needed to
assess speech quality, acoustic feedback and physical-device reliability; sample counters prove
delivery to the OS, not audible fidelity. The Windows 11 / Ryzen 7 5700X3D native duplex test
passed: 108,000 processed frames reached the output callback, live zero gain produced a zero
meter, a live mono update resumed output, and the worker released its permit after Stop. No
audio was saved. Release DSP measurements for 20 seconds at 48 kHz are 149.194 ms mono
(74.597 µs per 10 ms block) and 284.813 ms stereo (142.406 µs per block). These are synthetic
elapsed throughput figures, not game FPS or whole-app CPU usage. Logs are in ignored
`target/mic-monitor-{focused,focused-ui,native,ci}.log` and `target/mic-stereo-benchmark.log`.
Full `cargo xtask ci`, documentation checks and desktop/CLI release builds pass for the final
revision. Native 1440×900 review confirms the test row follows suppression/mono controls and
its listening/error help wraps without overlapping the button/meter. The first UI run reported
one playback-device error; its cause is not established. A clean 15-second CLI retry delivered
716,160 output frames, and a further 15-second desktop retry stayed active and stopped cleanly.
This is short-run evidence, not a claim that device failures cannot occur. Before/after/listening/
stopped screenshots are ignored in `target/mic-monitor-evidence/`. Final logs are
`target/mic-monitor-ci-final.log`, `target/mic-monitor-build-final.log` and
`target/mic-monitor-native-long.log`.

## Microphone noise suppression (M3.1a, Windows, 2026-10-27)

`cargo test -p clipcraft-record noise -- --nocapture` exercises chunk/rate invariance, partial
interleaved frames, finite normalized samples, a sample-aligned click, the initial model delay,
gain/mute through AAC, format validation, device recovery and a two-second backlog after overload.
The initial overload design could repeatedly fail after the retry wait; recovery now drains stale
input without DSP, releases its allocation and resumes fresh samples. That case has a regression.
Engine tests check default-off migration/persistence, mic-only wrapping in both track layouts,
unsupported-format warnings that preserve system sound and the existing push-to-talk controls.
UI tests check search, Tab/Enter enable/reset and the active-recording restart notice.

The generated recording test saves both mixed/separate layouts as MP4 and MKV, decodes through
external FFmpeg/ffprobe and checks noise energy, an unaffected system tone, track duration and
mute/zero gain. Its stationary background is about −33 dBFS RMS. A direct model probe found much
weaker reduction for loud, full-band noise around −21 dBFS; these tests do not establish voice
quality, universal noise removal or parity with Discord/Krisp.

The ignored `generated_noise_suppression_throughput` test processes 20 seconds of generated
48 kHz mono in release mode. On this Windows 11 Pro 26300 / Ryzen 7 5700X3D run it took
127.298 ms total, averaging 63.649 µs per 10 ms block (0.00636× real-time elapsed). It includes
test-source chunk/output allocations, after FFT warmup. This is elapsed synthetic DSP throughput,
not game FPS impact, a whole-app CPU percentage or a sustained-load guarantee.

The explicitly ignored `real_microphone_noise_suppression_delivers_bounded_finite_samples` test
opens the default input for five seconds, counts finite 48 kHz mono output and saves no sound or
device identifier. Run it explicitly only for a local microphone integration check. Neither this
check establishes listening quality or physical-unplug behavior. The raw settings meter in that
initial prototype is replaced by M3.1b's processed listening test above.

On Windows 11 Pro 26300 it passed with a 48 kHz stereo default input: 237,120 finite 48 kHz mono
output frames in 5.062 seconds; the largest read was 5,280 frames, below the 12,000-frame limit.
Logs remain in ignored `target/mic-denoise-{device,record,record-focused,focused}.log`.

`cargo xtask ci`, the documentation check and desktop/CLI release builds pass. An isolated,
non-recording native profile at 1440×900 verified the new switch, enabled mono guidance, raw-meter
help and `rnnoise` search. Native Tab focus is visible; the automated settings test reaches and
activates both the switch and its Reset action with Tab/Enter. Before/off/on screenshots are
kept out of Git in `target/mic-denoise-evidence/`. The user's active replay was not restarted.

## Discord CDN artwork verification (M10.2c, Windows, 2026-10-24)

`cargo test -p clipcraft-engine presence --lib` covers fixed CDN image selection, game-name
consent, unchanged owned IPC identity, unknown/hostile-label fallback, bounded fields and
the original product badge. Publisher image files are not bundled. The native offline preview
explicitly uses CUT rather than requesting remote images.

The explicitly ignored Windows `real_discord_game_artwork_preview` test requires saved presence
and game-name consent. It publishes three fixed **Artwork preview** labels (CS2/Dota/League),
holds each for at most 60 seconds for inspection, and requests clear/shutdown on exit or error.
All three received real-client acknowledgments. The original hidden-profile cause was Discord's
global **Share my activity** switch being off. The maintainer enabled it manually; real CS2,
Dota 2 and League cards then displayed their actual game icons plus CUT badge. Native profile/portal screenshots and
test logs remain in ignored `target/discord-art-evidence/` and `target/discord-art-*.log`.
The portal saved the CUT app icon and ClipCraft capitalization. A distinct viewer account is
still needed for audience/privacy verification. Full CI and desktop/CLI release builds pass.
The final release opened at 1440×900 and showed **Connected to Discord**, the CUT preview and
the explicit offline/hosted-art explanation in General. The bounded preview run finished and
its **Artwork preview** card disappeared before normal release startup. No capture was started
for these branding checks. Existing keyboard-consent regression tests pass in the full gate.

## CUT identity verification (M10.2b, Windows, 2026-10-24)

The `brand` tests check the C stem/opening and antialiasing at 16, 24, 32, 64 and 256 px,
reject hostile sizes before allocation, and compare committed SVG masters with shared geometry.
The UI regression draws two frames and verifies texture reuse without nested context locks.
Presence tests retain bounded opaque product/game asset checks.

The Rust `brand_assets` exporter generated the kit successfully; repeating it against the same
destination returned `AlreadyExists`. A native 1440×900 desktop review confirmed the CUT header
and visible Tab focus on Start clipping; Shift+Tab returned focus to the header. No recording
was started. Before/after and keyboard screenshots are kept out of Git in
`target/brand-ui-evidence/`; delivery artwork is in `target/brand-cut-final-20261024/`.
This is native app evidence, not a Discord profile or game-art verification.

The test strategy treats capture, media, and local library metadata as untrusted and checks failure
behavior as carefully as the happy path. Capture/replay, playback, persistent library metadata,
audio, and trim export have deterministic tests; platform evidence and real-device limits are
recorded in [`gaps.md`](gaps.md) and [`release-checklist.md`](release-checklist.md).

## Canonical gate

Run `cargo xtask ci` before completing an issue. CI calls the same command. It includes formatting,
strict Clippy, workspace tests, layer validation, asset attribution checks, and a `cargo-deny`
license audit. Keep focused test commands in crate docs, but do not create a second gate with
different pass criteria. Tests that create scratch folders remove them with a short retry, because a
capture worker can still be finishing a file when the app is dropped (this once made the Linux job
fail with "Directory not empty"); leftover temp files never fail a test.

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

## Linux speed measurements

`cargo run --release -p clipcraft-platform --example capture_time` times screen capture on this computer
(run it under a virtual display with a full-screen test video). `cargo run --release -p
clipcraft-record --example encoder_profile W H raw.yuv` times the software encoder on raw I420
pictures (`ffmpeg -i clip.mp4 -pix_fmt yuv420p -f rawvideo raw.yuv`). Both report this machine only;
they are not game-FPS evidence. `QP=26` fixes the quantiser and `FRAMES=n` limits pictures; `OUT=x.h264` writes
the stream so before/after outputs can be compared byte for byte and PSNR-checked with ffmpeg (give both
inputs `-framerate 60`). Encoder changes must stay bit-identical or show equal PSNR and size.

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
  Follow-up verified 2026-10-23 includes a 60.026-second native SDR 1080p60 generated-window run:
  3,589 returned pictures (0.333% deficit), 3,602 fresh WGC reads with no repeats, zero readbacks
  and 177.10 MiB peak sampled RSS. All saved follow-up files fully decode; this is a short
  throughput check, not a two-hour 60 fps or real-game pass. HDR compatibility remains enabled
  by default and is outside this native SDR evidence. The encoder-only `encoder_bench` example
  separates paced/unpaced hardware input from WGC/audio/UI; `capture_bench` also offers silent,
  synthetic and headless isolation modes. Require actual returned output and fresh-read counts,
  not input acceptance or constant-rate repeated images. Tests cover cached timestamps and
  failure precedence, bounded FPS hints, unsupported-only interval fallback, CPU readback
  admission, rational MF contracts, changed parameter sets and output-allocation bounds.
  Windows CI also verifies restored editor media with canonical fixture paths, matching production
  imports (including Windows' verbatim drive prefix) while retaining missing-source checks.
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


### M11.1e — Export performance and responsiveness (2026-10-23)

The full `cargo xtask ci` gate passes **532 tests across 32 targets**, formatting, strict Clippy,
layers, asset attribution and dependency/license checks. Five tests remain deliberately ignored,
including the new manual composition timing benchmark; no test was excluded to make the gate pass.
The first gate attempt ran out of workspace disk while linking duplicated generated binaries;
superseded build executables were removed and the gate rerun. One Clippy suggestion in the benchmark
argument parser was corrected. The final gate includes the small-window cancellation regression.

Frame tests retain the prior thumbnail/black-canvas/overlay algorithm as a reference. Matching,
smaller, larger, odd-sized, portrait and transparent RGBA pictures match across every colour effect
and black/white fade amounts; the matching opaque result retains the input allocation. The manual
release benchmark alternates old/new composition in one process. The new Rust
`sequence_performance` example generates its own 1080p60 source and records repeated full exports
and process memory observations. See [performance](performance.md) for workload, machine, ranges,
reproduction and limitations. Every before/after two-second MP4 was byte-identical. A six-second
series also completed, with three source cuts and one text layer; timing thresholds are not CI tests.
Existing generated tests verify fractional-cut audio/video clocks, real 30/60 FPS output, high
resolution/text, size targets, fake GPU fallback, cancellation, collisions and unchanged files.

Desktop regressions keep the single worker slot occupied until completion/cancellation is
acknowledged, reject edits/another export, allow Space and frame stepping, and report disconnected
workers. A deterministic clock exercises 1,000 progress callbacks in one second and permits only
ten progress wakes, then an immediate completion wake. Latest atomic progress still updates every
frame. An accessibility-bounds test uses the editor area inside a 960×640 desktop, verifies that
Cancel export is visible, clicks its actual target and checks cancellation without freeing the slot.

The native app ran on private Linux Xvfb `:84`, 1440×900 and 960×640, using isolated local settings,
recovered references, generated H.264/AAC media, three two-second cuts, text and software 1080p60
export. Space starts preview during a real export; Right pauses/steps without editing. The initial
small-window inspection found cancellation below the timeline; the final toolbar placement is
visible and a native click cancels an active job, removes its partial and publishes no new output.
Retry completes a six-second MP4 with **360 pictures, 1920×1080 at 60/1 FPS, stereo 48 kHz AAC and
13,635,063 bytes**. External FFprobe checked streams/duration and FFmpeg fully decoded it. Previously
completed outputs remain present, originals retain their SHA-256 hashes, and no partial remains.
Physical audio output is unavailable in this virtual display; encoded audio was checked in the file.

Before/after screenshots and generated media stay outside Git. Screenshots are under
`.clipcraft-local/export-performance/`: `before.png`, `after.png`, `export-step.png`,
`small-export.png`, `cancel-complete.png` and `export-complete.png`. The final app remains open on
the completed sequence. Physical GPU, Windows/macOS, real-game interference, long-GOP/long-run
memory, power and reference-machine latency measurements remain open. Lower editor controls can
still require a taller window; this change specifically keeps export cancellation reachable.


### M5.3 — Connected local workflow and failure recovery (2026-10-23)

The complete `cargo xtask ci` gate passes **535 tests across 33 targets**, including formatting,
strict Clippy, layers, assets and dependency/license checks. Five existing/manual/platform tests
remain ignored; no test was disabled for this change. The new connected engine scenario is:

```sh
cargo test -p clipcraft-engine offline_workflow_tests
cargo test -p clipcraft-engine interrupted_exports_reserve
cargo test -p clipcraft-export --test export trim_respects_existing
```

A generated moving source and 440 Hz stereo tone enter the production CaptureSession/recorder.
The test snapshots a game when Save is requested, stops capture before the save completes, then
registers that same MP4 in the library. It probes, generates a real thumbnail, favorites/collects,
reloads/rescans, plays pictures and consumes non-silent decoded audio. It imports the recording,
analyzes/exports the entire default range, makes a split and trim plus text through engine commands,
saves/restores a history-free recovery record, and exports 54 pictures at 60 FPS over 900 ms.
Cancellation and a disconnected source publish no output; reconnect/retry succeeds; an occupied
output stays byte-identical; source bytes, game, favorite, collection, probe and draft survive.
The test requires no display, sound hardware, external codec tool or network.

Three regressions failed against their previous implementations before the fixes:

- A recorder MP4's container duration included AAC padding past its video duration. Native import
  offered 2.368 seconds, while the playback reader's video timeline ends at 71/30 seconds. The
  original default range produced `EditorTimeline(Unsupported)` and `BadRange`. Import now uses
  bounded reader metadata; Send to editor schedules that resolution on its existing worker.
  Main-library container metadata is unchanged. Existing saved edit boundaries are not rewritten.
- A killed editor export left a `.partial` without a final MP4; automatic naming selected the same
  output, making retry report AlreadyExists. Naming now reserves final/partial/recovery paths,
  unreadable entries and broken links. It does not delete a possible live writer's file.
- Sample-copy trim used truncating File::create for its temporary and a replacing rename at the end.
  The new generated test verifies existing-partial refusal, raced-destination preservation, late
  cancellation and unchanged source. Exclusive creation, owned cleanup and no-replace hard-link
  publication now match the rendered exporters. Hard-link support is required for trim as well.

For native checks, start a **private** Xvfb display at 2240×1000 with TCP listening disabled and
run ClipCraft in an isolated XDG data/cache directory. The new optional Rust fixture window is:

```sh
cargo run -p clipcraft-platform --example workflow_game -- --private-display
```

It refuses an existing managed/client desktop, advertises only its own PID/window to the private
X server, paints an original moving rectangle and exits after at most 180 seconds. Use an exact
custom executable match for `workflow_game`, software H.264, 360p/30, 15-second replay, game-only
capture and no desktop fallback. No real game, game art or third-party recorder source is used.
The native run also used a private PulseAudio null sink/monitor and generated tone via local Unix
sockets. These are test/OS facilities, not shipped ClipCraft components or a hosted service.

The actual app on Xvfb `:85` automatically recognized the generated process/window. F8 saved a
640×360 H.264/AAC clip; the library displayed its game, cached thumbnail and favorite. The source
played through the ordinary ALSA/PulseAudio output adapter: monitor capture RMS was 0.034883
(normalized s16), with the test-tone producer already stopped. Send to editor, Add to timeline,
split at 0.9 seconds, an independent text layer and a custom export title all ran through the UI.
The fixed imported range produced decoded filmstrips and a real audio envelope.

SIGKILL during an actual editor export left an 8 KiB partial. Restart restored both cuts, text,
title and playhead paused with no undo/redo history. The original retry failure was captured;
after the naming fix, retry chose the next suffix and completed **107,550 bytes, 640×360,
30/1 FPS, 71 pictures, 2.366667 seconds and stereo 48 kHz AAC**. External FFprobe inspected it and
FFmpeg fully decoded it. It appeared in the ordinary library and played with text/pictures/audio;
its virtual output-monitor RMS was 0.029702. Native normal shutdown/restart again retained the
edit, and the source's favorite/captured game survived. SHA-256 checks confirm unchanged source
and interrupted-partial bytes. The completed export is a new file; the partial remains deliberately
reserved. There is no new automatic orphan-partial deletion policy.

Five app sessions, including normal and forced restart, ran under `strace -f -e trace=network`.
No Internet-addressed connect/send calls were observed; 31 local Unix connect calls included X11,
system audio and unavailable session-bus probes. Raw traces stay outside Git. Network namespace
creation failed because this sandbox makes UID-map configuration read-only; the audit is evidence
that this workflow made no Internet calls, not a physically disconnected/network-disabled release
run. Windows/macOS, physical speakers/microphones, real games/anti-cheat, Wayland, GPU impact and
long-run reliability remain unverified by this slice. No alpha/beta release gate is marked complete.

Screenshots stay under ignored `.clipcraft-local/offline-workflow/`: `waiting.png`,
`game-running.png`, `library-final.png`, `playback.png`, `before-full-clip-export.png`, `edited.png`,
`before-interrupted-retry.png`, `crash-restored.png`, `export-complete.png`, `export-playback.png`,
`after.png` and `restart-final.png`. Generated footage, audio, private configuration and syscall
logs are not committed. See [release checklist](release-checklist.md) for remaining platform gates.

### M7.3 — Local match-data adapters (2026-10-23)

`cargo xtask ci` passes **550 tests across 33 targets**, five existing/manual/platform tests
ignored, formatting, strict Clippy, layers, assets and dependency/license/source checks. The
focused commands are `cargo test -p clipcraft-engine game_state`,
`cargo test -p clipcraft-platform game_state`, and `cargo test -p clipcraft game_data`.

Engine cases cover CS2 fields/spectating, flat Dota versus nested spectator data, modern/legacy
League exact local-player selection, identity/token omission, missing-field replacement,
malformed/deep/oversized JSON and invalid numbers/text. Setup tests verify default-off settings,
shared commands, repeatability, preservation of existing config/partials and Unix key permissions.
Actual ephemeral loopback sockets test valid authentication, wrong token/AppID, HTTP errors and a
stalled body timeout. Other tests cover strict request framing, bounded chunked/length responses,
redirect refusal, stale status, disabled startup and shutdown. UI tests activate the two Valve
switches through Tab/Enter and verify clearing/reaping before replacement. A focus regression failed
before per-game stable IDs: removing CS2's live-details control displaced Dota's keyboard target.
It passes after the fix. Unsupported League cannot be enabled in the Linux UI; a saved enabled
setting can still be disabled.

The native app ran on a private Linux Xvfb `:86`, 1440×900, with isolated settings/cache and no
capture running. Existing/new switches were off; enabling CS2/Dota and setting up through the shared
CLI wrote only generated configs in temporary directories. Authenticated generated payloads produced
Mirage/competitive/8–6/18-9-4/73 HP and Dota/Axe/8-2-11/1500 HP summaries. Match details are expandable.
A wrong-token request returned HTTP 400, displayed an error, and the next valid update restored
live status. Native Tab/Shift-Tab/Enter toggled and re-toggled Dota; restarting retained switches.
Capture policy stayed unchanged. The shared CLI `watch 4` then received both generated games,
printed no key/account identity and released both ports on exit. Riot's published sample response
also parsed through `game-data parse league`, selecting Annie and discarding the account name.
The native file-picker/Steam installation/live-game workflow was not exercised; setup itself was
exercised through the same engine command from the CLI.

`cargo check -p clipcraft-platform --target x86_64-pc-windows-gnu` and
`cargo check -p clipcraft-platform --target x86_64-apple-darwin` both pass, including the OS TLS
adapter. macOS emits an existing unused Channel-variant warning in `priority.rs`; this is not a
native runtime check. No running CS2/Dota/League, target-OS TLS handshake, anti-cheat, physical
hardware or real-game performance is verified here. Windows/macOS live checks and all existing
capture release gates remain open. No Discord publication or saved rich clip metadata is claimed.

Build/linking exhausted the cloud workspace once; only superseded generated executable hashes
were removed, then the build and full gate passed. Before/after, live, details, keyboard and invalid
update screenshots stay outside Git in `.clipcraft-local/game-data/`. Generated configuration,
keys, public sample cache and JSON diagnostics remain in ignored/private temporary directories.
See [setup and protocol limits](game-integrations.md) and [ADR 0022](adr/0022-local-game-state.md).

### M10.2 — Optional Discord Rich Presence (2026-10-23)

The final `cargo xtask ci` gate passes **565 tests across 33 targets**, five existing/manual/platform
checks ignored, formatting, strict Clippy, layers/assets and dependency licenses/sources/bans.
Focused checks are `cargo test -p clipcraft-engine presence`,
`cargo test -p clipcraft-platform presence`, and `cargo test -p clipcraft presence`.

Three engine cases cover all consent combinations, browsing/editing versus active capture,
missing/stale/mismatched game snapshots, invalid Application IDs, hostile strings and UTF-8 wire
bounds. Ten Unix IPC tests use actual socket pairs with mocked Discord peers: handshake/READY,
activity/nonce acknowledgment, ping/pong, clear, malformed/oversized/truncated/error/close frames,
wrong nonce and timeout, no disabled discovery, coalescing/unchanged suppression, routine rate
limits, immediate privacy clear, reconnect, changed Application ID, opt-out during a delayed READY,
and normal shutdown. Expanding consent without changing activity sends no redundant update.
Two desktop tests use Tab/Enter for separate consent and applying an ID, and verify that disabled
or missing-ID settings start no worker. Fixture IDs are invented test data, never product defaults.

The actual native app ran on private Linux Xvfb `:87` at 1440×900, with isolated data/cache/runtime
roots. An isolated Unix socket mock at that runtime's `discord-ipc-0` recorded only synthetic
activity frames and replied with READY/ack. No real Discord account, client or profile was used.
Before opt-in and after enabling without an ID, the mock received zero connections. Typing the
fixture ID and activating Apply through Tab/Enter connected and sent only `Browsing clips`.
Game-name and match-context switches were enabled separately through the native UI.

A generated Rust `workflow_game` window was explicitly labelled Counter-Strike 2 by an isolated
custom test rule, then captured by the existing software recorder. Generated authenticated GSI
updates supplied Mirage/competitive/8–6, plus excluded health/money/KDA fixture fields. The mock
received the selected game/map/mode/score only; account identifiers, health, money, KDA and paths
were absent. Stopping updates cleared old match context while preserving game-name presence;
stopping the generated window cleared the game and returned to browsing. Opening Editor published
only `Editing clips`; the normal window close sent `activity: null`. These are synthetic protocol/
UI/capture checks, not evidence of CS2 gameplay or actual Discord profile behavior.

The CLI configure/settings round trip passed in a separate temporary data root. `presence preview`
returned only the allowed activity; malformed and >64 KiB stdin failed. Preview performs no IPC.
Windows (`x86_64-pc-windows-gnu`) and macOS (`x86_64-apple-darwin`) platform compile checks pass;
macOS retains an existing unused Channel-variant warning in `hotkey.rs`. Native Windows named-pipe,
macOS socket packaging, actual registered Application ID/profile/audience, Discord privacy settings,
live-game transitions and real-client shutdown/reconnect remain open. The maintainer has no
Application ID yet; the app shows setup instead of inventing a product identity.

No new test was ignored. Superseded generated executable hashes were removed to keep the cloud
workspace within disk capacity; source/media were preserved. Before/after, missing-ID, connected
mock, match and editor screenshots remain outside Git under `.clipcraft-local/discord-presence/`.
Private generated configs/mock frames and verification records remain under the temporary native
root. See [setup/exact fields](discord-presence.md), [ADR 0023](adr/0023-discord-presence.md) and the
[release checklist](release-checklist.md). Rich per-clip match metadata is not implemented here.

### M10.2a — Shared identity and Windows IPC follow-up (2026-10-24)

The maintainer supplied public ID `1563496212554846279`, now included in ordinary builds. New
engine regressions cover missing/empty release values, valid explicit overrides and malformed
nonempty values. The focused presence policy suite passes all six tests.

The first real Windows check reproduced a transport failure: an empty nonblocking byte-pipe read
returned zero and was treated as EOF before READY. The adapter now queries bounded available bytes
with `PeekNamedPipe`, preserving genuine disconnect errors. A local real-pipe regression covers
empty → data → empty → peer close without Discord or account data. The explicit live check then
received READY and `SET_ACTIVITY` acknowledgment from the desktop client, with basic browsing
context and saved presence consent, followed by bounded clear/disconnect. No raw payload or account
data was logged. Run it only with Discord open and presence enabled:

```text
cargo test -p clipcraft-platform --lib real_discord_accepts_configured_activity -- --ignored --nocapture --test-threads=1
```

Artwork has been generated with the existing Rust exporter under ignored
`target/discord-presence-assets-20261024`; uploading it and checking actual profile rendering,
audience settings, opt-out visibility and macOS remain separate manual gates.

The final desktop/CLI release build, `cargo xtask ci` and `cargo xtask docs-check origin/main`
pass. The full gate log is `target/discord-presence-ci.log`; the release log is
`target/discord-presence-release-build.log`. The live check was explicitly run outside the
restricted environment against the user's running Windows Discord client.

### M10.2a — Medal-inspired card and shared setup (2026-10-23)

`cargo xtask ci` passes **568 tests across 34 targets**, five existing ignored checks, fmt,
strict Clippy, layers/assets and dependency checks. No new ignored test or dependency is added.
The shared-ID resolver covers valid/invalid fallback and explicit-override priority. Focused engine
tests also ran with a fixture `CLIPCRAFT_DISCORD_APPLICATION_ID` and an invalid build value, checking
that the actual target uses the shared identity only when valid. No test ID is shipped as a default.
The disclosure matrix now checks badge selection, `Clipping <game>`, readable Mirage/Competitive
context and forbidden private fields. Bounded original art rejects hostile keys/canvas sizes.

Native Linux Xvfb :87, 1440×900: inspected before/after settings screenshots; normal controls have
no ID form and show an identity-independent local preview. Advanced controls appear under the
explicit heading, before General's final advanced toggle. Typing a fixture override and using
Tab/Enter on Apply connected to an isolated Unix mock. Without an effective ID the enabled app
made zero mock connections. A generated Rust `workflow_game` window (custom test rule labelled
Counter-Strike 2, software capture) produced the CS2 large/ClipCraft small badge in both the native
preview and actual IPC JSON. This is synthetic capture/IPC, not a CS2 or Discord account test.
Normal close sent an acknowledged null activity. Automated egui keyboard checks cover separate
consent, advanced Apply and reset; accessibility checks verify preview text and no normal ID form.

CLI preview checks without an ID cover CS2/Dota/League/unknown-game badges, absent-game consent
and disabled/null output. The Rust exporter produced four 1024px PNGs and license/provenance;
a second run refused the existing destination and preserved all PNG hashes. Preview and export
share the same bounded renderer. Inter SemiBold matches the pinned FilmCraft font by SHA-256.
Windows/macOS platform compile checks pass (the pre-existing macOS `hotkey::Channel` warning
remains); documentation gate passes. Actual registered application/art rendering, Discord audience,
real-client reconnect/exit and Windows/macOS runtime remain open.

Ignored evidence: `.clipcraft-local/discord-presence/medal-before*.png`, `medal-after*.png`,
`medal-game-card.png`; original generated art under `.clipcraft-local/discord-assets/`;
private fixture configuration/frames under `/tmp/clipcraft-presence-native`. Public Medal support
screenshots were inspected as layout references only, never shipped. See [setup](discord-presence.md)
for the one-time maintainer registration/upload/build-variable requirement.

### M11.1f — Latest-picture mailbox and seek publication (2026-10-25)

`cargo xtask ci` passes **577 tests across 34 targets**, with five pre-existing ignored/manual
checks: formatting, all workspace targets, strict Clippy, layer/asset and dependency checks.
`cargo xtask docs-check ba03a60` passes. Rust 1.99.1 on Debian 13; no new dependency, unsafe,
ignored test or source port. Build prerequisites were restored outside tracked source files.

`cargo test --locked -p clipcraft-player` passes **69 tests**, including nine new cases. A
10,000-frame synthetic burst retains just the last 4×4 RGBA frame (64 payload bytes) and preserves
opening → latest frame → terminal order. Seek, step and replay clear queued old frames/Ended;
a barrier-controlled late completion is rejected and a current-revision result still publishes.
Four concurrent callers enqueue 2,048 seek commands with strictly ordered revisions. Fatal errors,
close, disconnected command receivers, poisoned mutex and revision exhaustion are covered, as are
zero/overflow/oversize dimensions before conversion. Poison handling also wakes a paused worker
with Shutdown, preventing retirement from waiting on a permanently poisoned mailbox.

A generated one-second 64×48 H.264 MP4 plays completely while the test deliberately never polls;
it then returns exactly Opened, the true last picture, and Ended. A subsequent seek shows the
requested frame without a stale end marker, and cancellation acknowledges worker retirement.
Existing player tests cover exact seeks/stepping, replay/loop/speed, audio selection and sound-clock
behavior. Desktop egui regressions cover prepared-source promotion at a cut, rapid seeks/reorder/
reopen worker bounds, editor keyboard transport and recovery. No UI layout changed; these tests
are not a native display/sound-device or hardware-performance run.

The limit applies to retained display results, not decoder/GOP caches, UI-held frames, in-flight
conversion, control/audio channels or process RSS. No shared cross-job priority scheduler,
speculative prefetch, preview proxy, GPU compositor or faster export is claimed. See
[ADR 0024](adr/0024-bounded-playback-delivery.md) and the [performance accounting](performance.md).
Local logs: `/tmp/clipcraft-bounded-player.log`, `/tmp/clipcraft-bounded-ci.log` and
`/tmp/clipcraft-setup-verification.log`. Tests create synthetic media in disposable directories;
no user clips or downloaded Concat fixtures are read or changed.

### M11.1g — Playback-first decode admission (2026-10-25)

The final `cargo xtask ci` passes **584 tests across 34 targets**, five pre-existing ignored checks,
formatting, strict Clippy, layers/assets and dependency licenses/bans/sources. The player target
passes **76 tests**. No UI layout, engine command ID, media schema, dependency or first-party unsafe
changes. All new work is independent Rust using existing FilmCraft interfaces.

Six isolated-coordinator tests establish: at most three active stages and one background stage;
two foreground stages can run while background holds its slot; a queued foreground request wins
the next available slot; cancellation unregisters even while all slots stay occupied; callbacks
run outside the coordinator mutex; panic/early-return releases permits; waiting capacity/poison
fails explicitly; a 48-background-waiter budget cannot fill the last 16 foreground registrations.
Eight concurrent caller threads complete 800 mixed admissions without exceeding active/background
counts, then return all counters to zero. Cancellation uses a 20 ms polling interval, not a tested
real-time/OS scheduling guarantee. Tests use private coordinators, so poison/capacity fixtures do
not corrupt the shared runtime coordinator or other tests.

A generated one-second 64×48 H.264/PCM MP4 plays with the normal audio worker while three background
threads each generate three thumbnails and timeline picture/waveform analyses. Playback reaches
Ended, results retain the requested sizes and audio presence, workers complete, and original bytes
remain unchanged. Existing audio pitch/clock/track, seek/step/loop/shutdown, malformed media and
packet cancellation cases pass. Full desktop regressions also pass prepared-source promotion,
rapid seek/reorder/reopen worker bounds, keyboard transport and recovery. These are generated
portable tests, not a native display/audio-device, GPU or real-game measurement.

Player open/video and audio blocks are foreground; uncached thumbnail generation and timeline
open/pictures/waveform chunks/MP4 packets are background. Analysis yields between chunks; audio
releases admission before output-buffer waits. Stop and superseded video waits abandon queued
admission; analysis/audio use their existing cancellation/close state. Already-running codec/file
calls cannot be preempted. Metadata-only source-info and UI cached-JPEG loading remain ungated;
export/recording are outside this coordinator. Thumbnail's legacy API has no cancellation argument.
No CPU-core/thread/RSS bound, before/after latency, GPU acceleration or stuttering claim is made.
See [ADR 0025](adr/0025-cooperative-decode-admission.md) and [performance](performance.md).
Logs are local: `/tmp/clipcraft-priority-player.log`, `/tmp/clipcraft-priority-ci-final.log`.

### M11.1g retest and M11.1h shortcut correction (2026-10-25)

The user-requested retest began at `e994e1a`: full CI again passed 584 tests/34 targets (five
existing ignored). The six isolated scheduler tests passed **50 repetitions** and the generated
mixed playback/sound/thumbnail/waveform regression passed **10 repetitions**, without failure.
These are repeated executions, not additional unique tests or a hardware benchmark.

Built and opened the actual desktop app on isolated Linux Xvfb :89, 1440×900, with private
XDG data/cache/runtime roots and two generated H.264/AAC MP4 clips (1280×720/8s and 640×360/6s).
Checked library thumbnails, advancing picture/clock, pause, rapid number seeks, frame step,
Send to editor, adding the source to the timeline, editor playback, decoded filmstrip/waveform,
ruler seek and normal close/relaunch with draft recovery. No source/user media was imported from
outside the disposable fixture directory. Fixture hashes remained unchanged after the corrected
native run. Actual audio-device output was unavailable (ALSA default device rejected); the app
reported video-only playback and continued on its wall clock. Generated audio/pitch/clock tests
passed, but no audible speaker/microphone or physical GPU/game test is implied.

Native rapid 1,8,2,9,3 input exposed M11.1h: the old numeric shortcut scan could apply 90% last
within a batched UI frame. Added a one-frame egui regression that failed before correction (9s
instead of 3s on a 10s synthetic description), including final-zero and repeated-key cases.
The handler now takes the last pressed digit in event order. Native repeat followed by one frame
step shows 2.433s/frame 73 rather than the prior 7.233s/frame 217 on the 8s fixture. Existing
text-focus/popup guards and all other transport controls retain their original paths.

After correction, `cargo xtask ci` passes **585 tests across 34 targets**, five existing ignored,
plus fmt/strict Clippy/layers/assets/dependency checks; docs gate passes against `e994e1a`.
No new dependency, unsafe, layout, command/schema or Concat source port. Screenshots remain ignored
under `.clipcraft-local/playback-priority-test/` (`playback-advancing.png`, `editor-playback.png`,
`playback-seek-step.png` before fix and `seek-fixed.png` after). Logs are
`/tmp/clipcraft-priority-retest-ci.log`, `/tmp/clipcraft-priority-repeat-tests.log`,
`/tmp/clipcraft-seek-before.log`, `/tmp/clipcraft-seek-fix-ci.log`; private fixture/runtime files
are under `/tmp/clipcraft-playback-retest`. Only owned test processes were stopped.

During final publication, main advanced to `755c999` (merged branding/SVG icons and Windows
performance work). Rebased only the seek correction onto that revision, preserved its changes,
and repeated the full gate: **602 tests across 36 targets pass**, five existing ignored, plus
strict Clippy/format/layer/asset/dependency checks. Rebuilt that exact combined app and repeated
native rapid-seek/frame-step and restored-editor playback checks; 2.433s/frame 73 remains correct,
filmstrip/waveform and the new CUT/SVG assets render, and source hashes stay unchanged. Latest
screenshots: `latest-seek-fixed.png`, `latest-editor-playback.png` in the same ignored evidence
directory. Final logs: `/tmp/clipcraft-latest-retest-ci.log`, `...-build.log`. Documentation gate
passes against `755c999`. No newer Windows runtime/hardware evidence is inferred from this Linux run.

## M11.1i — Shared compositor (2026-10-25)

Started from fetched main `9d99de8`; no Concat code/shader port or first-party unsafe. The fixed
original WGSL is embedded in the Rust binary. Pinned wgpu 27.0.1/pollster 0.4.0 and their native
backend dependencies pass the unchanged license/source/bans audit with one package/version-scoped
CC0 allowance for Naga's `hexf-parse 0.2.1`; see ADR 0026.

`cargo xtask ci` passes **610 tests across 37 targets**, with five existing ignored/manual cases:
formatting, all targets, strict Clippy, layer/asset checks and dependency audit. No new ignored test.
`cargo xtask docs-check 9d99de8` passes. Rust 1.99.1/Debian 13, two build jobs, incremental off.
Local `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` omit debug symbols only; the
normal hot-loop/dependency optimization levels are unchanged. The first default-debug link ran
out of workspace disk; obsolete generated hashed test executables were removed, preserving source,
fixtures and evidence, then the complete gate reran under these profile overrides.

Coverage includes aspect-fit portrait/ultrawide/no-upscale geometry, exact RGBA backing length,
hostile dimensions/NaN/positions/mask shape/count/combined byte budget, no-op ownership retention,
independent TextRaster alpha reference, and preview/export canvas/look/fade/text at identical Tick.
Worker tests reject failed/malformed transformed frames without touching sources and use a barrier
to prove a superseded renderer failure cannot reject a newer seek. Editor tests cover paused recipe
refresh without audio activation, prepared-next-source promotion, continuous/discontinuous same-file
cuts, rapid seek/reorder/retirement budgets, text interaction and keyboard transport. Initial editor
integration exposed prepared/continuous-cut regressions; actual initial recipes and visually
unchanged plain-cut handling fixed them before completion.

### Actual software GPU proof

Signed Debian `libvulkan1 1.4.309.0-1` and `mesa-vulkan-drivers 25.0.7-2+deb13u1` were extracted to
`/tmp/clipcraft-vulkan` without system writes. With `LD_LIBRARY_PATH` pointing to its lib directory,
`VK_DRIVER_FILES` to its `share/vulkan/icd.d/lvp_icd.json`, and owned `XDG_RUNTIME_DIR` and
`MESA_SHADER_CACHE_DIR`, run:

```sh
cargo run --locked -p clipcraft-export --example compositor_verify
cargo test --locked -p clipcraft-export compositor -- --nocapture
```

The example **requires** a real software adapter and fails if unavailable; it never silently counts
CPU fallback as GPU success. Actual adapter: **llvmpipe (LLVM 19.1.7, 256 bits), Cpu, Vulkan**.
All **96** cases passed within one 8-bit channel level: four source/canvas shapes, all eight looks,
three fade modes, source alpha and two overlapping/cropped text masks. Transfer-inclusive stage
timings are in [performance.md](performance.md), not an end-to-end or physical-GPU claim. A real
software device was destroyed; the automatic-path context helper retired it and CPU completed.
On hosts without a software adapter the unit test reports only CPU fallback; the strict example
must run separately to establish GPU execution. Logs: `/tmp/clipcraft-gpu-verify.log`,
`/tmp/clipcraft-gpu-compositor-tests.log`, `/tmp/clipcraft-gpu-ci.log`.

### Native UI and generated export

The rebuilt application ran on a private Xvfb/GL desktop at **1440×900** and **960×640**, with
isolated local settings/draft/media and Discord/capture disabled. Baseline and final screenshots
are under ignored `.clipcraft-local/playback-priority-test/`: `gpu-before-look-1440x900.png`,
`gpu-after-look-matching-tabs.png`, `gpu-paused-invert.png`, `gpu-text-drag.png`,
`gpu-keyboard-stepped.png`, `gpu-export-dialog.png`, `gpu-export-status.png` and
`gpu-editor-960x640.png`. They show existing CUT/SVG styling, a generated colour pattern with Sepia,
white fade and lime lower-third text, paused effect refresh, text drag handles and export controls.
Mouse drag moved the baked text; Ctrl+Z restored it. Space play/pause, Right frame step and
Tab/Shift+Tab focus worked. Native display used the ordinary CPU fallback; the separate diagnostic
above proves software Vulkan. Physical audio output is unavailable in this container; no audible
hardware check is claimed.

A real native editor export completed: **8.000 s, 1280×720, 30 fps, H.264 + stereo AAC,
4,014,134 bytes**, reported Software. The external ffprobe oracle confirmed streams/timing and
ffmpeg decoded the whole output without errors. Extracted frame 63 (2.100 s),
`gpu-export-frame.png`, matches the preview's Sepia/text placement by inspection; lossy codec/UI
scaling are not a bit-exact parity test. Both generated source SHA-256 hashes stayed unchanged.
Temporary scripts/oracles and media are test-only, not product dependencies or tracked assets.

Physical GPUs, Windows DX12/macOS Metal runtime, power, real-game contention and end-to-end speed
remain open. Decode/native YUV pre-reduction/RGBA and glyph rasterization remain CPU work; GPU
uploads/readbacks remain. Native driver calls are not preemptible. No decoder pool/proxy or release
platform gate is closed by this task.

## M11.1j — Unused-code cleanup (2026-10-25)

Based on fetched main `62bed30`; [issue #13](https://github.com/nanox333/clipcraft/issues/13).
Removed obsolete UI effect/fade/text painting and the unused text texture LRU. The UI now keeps
only selected-active-text key/canvas/size metadata; a bounded raster is temporary when this geometry
changes. Unselected layers need no UI raster. Shared worker composition, including CPU fallback,
its per-worker pixel budget, FilmCraft text placement and normal frame texture uploads remain.

Removed two uncalled game helpers, the never-integrated ClipRecorder audio-timeline module and
unused `checked_frame_rate`/`SampleRate`/`TimeError` wrappers. Exact frame/sample/hour-long clock
regressions still test FilmCraft Tick directly. Deleted tests exercised only the removed APIs;
no active workflow regression was removed or ignored. The source port/provenance is recoverable
from Git history; current docs no longer claim it is integrated or present. The public text blend
method stays because the brand asset exporter and independent compositor reference tests use it.
Platform fault-injection/separate-display APIs remain for integration testing.

`cargo xtask ci` passes **602 tests across 37 targets**, five existing ignored/manual cases:
formatting, workspace/all targets, strict Clippy, layer/asset and dependency/license/source checks.
The test total is lower because the dormant utility and unused wrapper tests were deleted, not
because active checks were skipped. `cargo xtask docs-check 62bed30` passes. Rust 1.99.1 on Debian
13, existing optimized development policy with debug symbols disabled locally for disk capacity.
`cargo-machete 0.9.2 --with-metadata` reports no unused dependencies. `windows-core` is explicitly
listed as a false positive: `windows::core::implement` generates `::windows_core` references for
Media Foundation callbacks; it cannot be removed based on Linux lexical scanning.

The geometry regression checks selected-layer-only caching, unchanged key allocation/bounds during
drag and viewport resize, and cache release on deselection/close. Existing tests cover text layers,
paused recipe refresh, prepared and continuous/discontinuous cuts, keyboard transport, source-safe
export, malformed media and cancellation. These tests do not establish new physical GPU/platform
runtime or speed evidence.

Native private Xvfb/GL smoke at **1440×900** and **960×640** used isolated generated clips/draft,
Discord disabled and capture stopped. Ignored screenshots under `.clipcraft-local/playback-priority-test/`:
`cleanup-before-1440x900.png`, `cleanup-after-1440x900.png`, `cleanup-selected-handles.png`,
`cleanup-text-drag.png`, `cleanup-keyboard-1440x900.png`, `cleanup-after-960x640.png`.
The Sepia/white-fade/lime-text draft rendered without duplicate glyphs; selecting/dragging its
handle moved the baked text, Ctrl+Z restored it, Space play/pause, Right frame-step and
Tab/Shift+Tab focus worked. Both original generated source SHA-256 hashes stayed unchanged.
Physical sound hardware is unavailable; this was the ordinary CPU-composition fallback, not a
physical-GPU benchmark. Log: `/tmp/clipcraft-cleanup-ci.log`; dependency scan:
`/tmp/clipcraft-cleanup-machete.log`. No test media, scripts or screenshots are tracked/shipped.

## M2.P2 — Idle/library responsiveness (2026-10-25)

Based on fetched main `8022671`; [issue #14](https://github.com/nanox333/clipcraft/issues/14).
No new dependency, first-party unsafe, schema/index format or ignored test. Rust-only native/offline
engine commands remain shared with CLI. [ADR 0027](adr/0027-responsive-library.md) records bounds,
optimistic edits, ordered retry and shutdown limitations.

The static-focus regression was run with the old self-repaint request temporarily restored:
it failed with `0ns` next repaint. Restoring the fix passes with a focused response and no immediate
next repaint. Controlled generated 10,000-entry tests compare cached paths, counts and game groups
with the previous reference for all categories, Unicode queries, missing dates, both sorts and
metadata invalidation. Repeated unchanged preparation reuses indices. Editor search preserves
insertion order, reuses its Arc indices and observes media Arc/make_mut changes.

Virtual-grid tests build fewer than 40 cards in a 1200×700 viewport with 10,000 entries. Home/End
reach both ends with stable IDs. Pointer activation focuses cards; Tab/Shift+Tab cross a virtual
row without egui double-advancing focus. Native testing exposed the pointer-focus detail before
completion; the fix is covered by that regression. Text fields/popups retain normal key handling.

Writer tests use controlled barriers for delayed work/admission/polling and exercise count/byte
bounds (including active work and replies), ID exhaustion, poisoned locks, terminal join failure,
explicit discard, and failure/panic → pause → ordered retry → drain on shutdown. A real engine
round trip delays a persisted scan reply while UI favorites/collections, a probe batch and recorded
label register behind it; the old reply does not undo newer UI state, and all fields survive index
reload. Generated sources are byte-identical. Normal close cancels closure until accepted writes
are acknowledged, then requests Close. Existing tests cover unindexed scan metadata preservation,
latest folders, probe generations, saved-game tagging, failed persistence and source safety.

The native app ran at **1440×900** and **960×640** on the private Xvfb/GL desktop. A temporary index
containing exactly **5,000 generated/fixture entries** exercised visible rows, Home/End, search/sort
controls and favorite toggling. A favorite survived normal shutdown in the index. The original
small fixture index was restored after closing; both original media SHA-256 hashes were unchanged.
Screenshots under ignored `.clipcraft-local/playback-priority-test/` include
`performance-idle-fixed-focus.png`, `performance-library-1440x900.png`,
`performance-library-5000-top.png`, `performance-library-5000-end-fixed.png`,
`performance-library-5000-home-fixed.png`, `performance-library-5000-960x640.png`.
Initial End testing identified the pointer-focus omission; final screenshots use the corrected
card focus. Cloud process CPU and actual cache-helper timings/methods are in [performance](performance.md).
These are not physical-GPU/audio, Windows/macOS runtime or gaming-PC measurements.

`cargo xtask ci` passes **614 tests across 37 targets**, with five pre-existing ignored/manual
checks: formatting, workspace/all targets, strict Clippy, layer/asset and dependency audit.
`cargo xtask docs-check 8022671` passes. Logs: `/tmp/clipcraft-ui-performance-ci.log`,
`/tmp/clipcraft-ui-performance-docs.log`. The native build uses the same normal development
optimization policy with local dev/test debug symbols disabled to fit disk; no release flag changed.

Normal close can wait on a non-preemptible filesystem operation. Failed-close discard and force
quit/crash can lose unacknowledged **metadata**; source clips and the last published index remain.
Startup index reads, JPEG loading and settings writes have not all moved off the UI. Remaining
performance proposals are not declared complete by these tests.
