# Known gaps

## Multiple shortcuts, first-run setup and Windows installer (M5.1a)

Each action accepts five keyboard bindings with a + button, individual removal, collision feedback
and legacy-settings migration. PTT holds are combined across alternatives. Native registration
uses unique binding IDs; registration is rebuilt only when shortcuts change. Global availability
still depends on OS permissions and other applications owning the shortcut.

Fresh profiles see four setup steps for local storage/replay, capture/audio and shortcuts. Setup
uses a draft; no capture/audio/presence worker starts during it. Finish or Skip saves completion
with the choices through the shared settings command. Existing preference files skip setup.

The software H.264 encoder's Speed preset is 1.3-3x faster than FilmCraft's original (see `docs/performance.md`); on a scrolling-UI clip it matches x264 `veryfast`'s speed (about 7 ms per 1080p picture on one thread), but real gameplay was not measured and keyframes still stall, so a hardware encoder stays the recommended path.
ClipCraft asks Windows for borderless capture access once per run (ADR 0033); Windows answered Allowed on the development machine, but the yellow frame was not checked on screen against a captured game.
The first-run setup and the installer/uninstaller use the app's own look (Inter, charcoal and lime); development builds of the installer accept `--preview-working` and `--preview-done` to show those pages without installing.
`cargo xtask package` also produces an offline Windows `ClipCraft-Setup-<version>.exe`, written entirely in Rust, and ships the CLI as a separate archive.
Optional update check and one-click update (ADR 0032) are built and checked against GitHub's real release API and redirects; a full update against a release that carries a `ClipCraft-Setup` asset has not run because none is published yet.
It installs per-user, makes Start menu/optional desktop links and registers an uninstaller.
Upgrades retain previous program versions (maximum eight); uninstall removes owned program files
and retains unknown files, preferences and recordings. A temporary uninstaller executable can
remain in the Windows temporary directory for OS cleanup. No automatic updates, code signing,
macOS installer or clean second-machine/SmartScreen verification is claimed. The portable zip
and Linux archive remain available. See ADR 0031 and the release checklist for scope/evidence.

## Microphone noise suppression (M3.1a, 2026-10-26)

Optional offline RNNoise suppression is implemented. The native Audio setting defaults off,
persists through the shared settings command and takes effect after clipping restarts. Only the
microphone is processed; game/system sound bypasses it. Stereo is preserved using separate
models per channel; selecting **Microphone is mono** explicitly folds it. The pinned pure-Rust
model needs no downloads or network access.

M3.1b replaces the raw meter with **Mic Test** listening through the default output, after
suppression, volume and mono/channel settings. It uses the recorder's input factory and PCM
conversion, updates live, and stops on Stop/leave/hidden row/error or after 60 seconds. The
test is independent of recording/PTT switches, has one bounded worker and 200 ms output queue,
and saves no sound. Generated PCM, queue/error/lifecycle and keyboard tests cover the new path;
the Windows native duplex check delivered 108,000 processed frames to the output callback and
passed live zero-gain/mono updates and worker cleanup without saving sound. Full `cargo xtask ci`
passes, along with final native UI and desktop/CLI release builds. One playback-device error
occurred in manual testing; clean 15-second CLI/desktop retries pass, with the cause unproven.
Physical-device reliability and subjective speech quality remain open. Optimized synthetic processing costs about 75 µs per 10 ms
mono block or 142 µs for stereo on this Ryzen 7 5700X3D; game impact remains unmeasured.

Twelve deterministic DSP tests pass, including split/partial frames, 8–192 kHz conversion,
sample alignment, hostile samples, downstream gain/mute and recovery from a two-second backlog.
Generated mixed/separate microphone/system tracks save and decode in MP4/MKV with timing and
noise-energy checks. At roughly −33 dBFS RMS, generated steady background noise is reduced;
loud full-band noise around −21 dBFS was much less reduced in a direct model probe. Release-mode
initial mono-only synthetic throughput was 127.298 ms for 20 seconds on a Ryzen 7 5700X3D. These are not speech-quality
or game-FPS measurements.

A Windows 11 default-input integration run delivered 237,120 finite mono output samples in
5.062 seconds from a 48 kHz stereo device in the initial mono prototype; no audio was saved. Real speech listening quality,
physical-device unplug/recovery, echo cancellation and game FPS impact remain unverified. Native
1440×900 screenshot/search review, automated Tab/Enter enable/reset, full `cargo xtask ci` and
desktop/CLI release builds pass. This is not a claim of Discord/Krisp parity.
See [ADR 0030](adr/0030-microphone-noise-suppression.md).

The selected **A / CUT** branding has shared Rust geometry, scalable monochrome masters,
small icon exports, light/dark wordmark lockups and a Discord product badge. The native icon
and header use the same mark. CS2, Dota 2 and League now reference fixed Discord CDN icons;
publisher image files are not bundled. Other games use CUT. CDN references can become stale,
and the offline preview shows CUT rather than downloading game art. The CUT icon is uploaded;
the real Windows profile rendered CS2, Dota 2 and League icons with the small CUT badge after the maintainer enabled
Discord's global activity sharing. Other-account audience checks remain open.
See the [brand guide](brand.md) for reproduction and constraints.
Verification: small-size/hostile-input and texture-cache regression tests, native 1440×900
screenshots, Tab/Shift+Tab checks, exporter overwrite refusal and `cargo xtask ci` passed.

CI note: the Linux job once failed on a scratch-folder cleanup race in a capture test; cleanup now retries and
the Windows draft-recovery test uses canonical paths. Both jobs passed their test gates on the merged `main`.

Linux re-check (2026-10-26, virtual X display, 4 cores, no GPU): the desktop app started, Start clipping armed capture, F8 saved a 14 s MP4 that decodes with ffmpeg and appears in the library; `clipcraft-cli capture record` also produced a valid clip. The H.265 and tray changes had left `main` failing the Linux lint (code only used on Windows); that is fixed. With the software encoder only, 1080p60 from a 1440x900 desktop dropped about 55% of frames (about 17 encoded pictures per second on 4 cores); the app shows this as a red warning. A graphics-card encoder on Linux was not available to test.

CI fix (2026-10-28): the borderless-capture change left the Windows-only graphics module without its Windows gate, so `main` stopped compiling on Linux; the gate is restored and `cargo xtask ci` passes on Linux. Six Windows app tests (editor preview readiness, game-exit capture) failed in the same run and have not yet been reproduced or explained; they are watched on the next Windows run.

Windows lint (2026-10-28): the shared picture-conversion thread helper was only used by the Linux encoder, so the Windows lint failed on unused code; the Windows Media Foundation processor path now uses it too (up to four threads, never more than the machine has, instead of a fixed two). The Windows lint over the whole workspace is clean when run from Linux (`--target x86_64-pc-windows-gnu`); the Windows speed effect is unmeasured.

Test stability (2026-10-28): the engine test that a save with no post-roll happens "at once" asserted a 2 s wall-clock bound and failed once on a loaded Windows CI runner (the first run of the merged encoder work; Ubuntu passed). It now asserts the post-roll is zero and bounds the time by the 3 s post-roll it must not wait out.

Hardware encoder fallback (2026-10-28): when the graphics card's encoder was unavailable at the start or stopped mid-session (for example while a game took the card), the recorder switched to the processor encoder and stayed there until the picture size changed or the screen went silent, which keeps fans spinning and drops frames. It now asks the card again after 20 s (doubling after each failure, a card that keeps stopping is asked less often and given up on after eight failures); when it answers, the stream restarts on the card. Limit: the replay buffer starts over at that switch, because its pictures belong to the old stream. Covered by recorder tests with a stand-in encoder that fails, then recovers, and one that keeps failing; not observed on a real Windows machine under game load.

Linux speed (2026-10-26, 4-core cloud VM, Xvfb): X11 capture now uses MIT-SHM shared memory (a memory file the server draws into; no `unsafe`), with the plain `GetImage` path as fallback: 1080p capture went from 15.1 to 3.0 ms per picture and 4K from 40.8 to 14.3 ms (25 -> 70 fps ceiling). Clips that keep the screen's shape are no longer enlarged beyond the screen (a 1440x900 desktop at the default "1080p" setting was being upscaled to 1728x1080, about 44% more pixels for the software encoder); `crates/clipcraft-record/examples/encoder_profile.rs` measures the encoder: at 1440x900 it reaches about 53 pictures per second on 2 threads, against about 29 at 1728x1080. Hardware encoders (NVENC, VA-API) were not available to test. Not measured on a real desktop with a game running.

Linux hardware-encoder input (2026-10-26): the RGBA-to-NV12 conversion that VA-API (and the Windows processor-memory path) runs on the processor before upload is rewritten to vectorise (whole-pixel integer masks, fixed 16-pixel blocks), bit-identical to the old formula (random and extreme-picture tests) and 2.5x faster: 1080p 10.1 -> 4.1 ms on one thread, 3.0 -> 1.6 ms on four (VM, `cargo test --release -p clipcraft-platform nv12_speed -- --ignored --nocapture`). It now uses up to four threads, never more than the computer has. Not done, because it cannot be checked without a graphics card: zero-copy DMA-BUF input and graphics-card scaling through the VA-API video-processing entry point (the Windows `SurfaceFit` equivalent), and overlapping conversion with the encode of the previous picture; VA-API pictures are still copied through a mapped surface and waited on one at a time.

Linux compatibility (2026-10-26): [docs/linux.md](linux.md) lists what works on X11 and Wayland, what each needs, and what is unverified. New: global hotkeys on Wayland by reading the keyboards (evdev; needs the `input` group or an optional udev rule, `packaging/linux/`), with X11 grabs as the fallback; the Wayland portal's chosen screen is remembered between runs (`restore_token`); `diagnose` reports the session type and hotkey access. Verified against a real PipeWire server with a fake desktop portal and with synthetic device files; **no real Wayland desktop or physical keyboard was tried**, and the download needs glibc 2.35 or newer and ALSA.

Encoder ownership (2026-10-26): the pure-Rust H.264 encoder (and its bit writer) now lives in `crates/filmcraft-h264enc` and `crates/filmcraft-bitstream`, vendored from FilmCraft commit `6c3948a` with licences and provenance kept (ADR 0029), so ClipCraft can tune it for screen recording. Encoder speed work (2026-10-26, Speed preset): static-content skip, zero-residual shortcut, deblock/intra/SATD/chroma-MC fast paths made pictures 1.3-2.1x faster on 1440x900 test clips at the same quality (desktop 23.1 -> 10.9 ms, scroll 48.9 -> 37.9, pan 51.5 -> 37.4, rotate 42.0 -> 27.8, game-like 55.2 -> 39.6 ms on 2 threads; table in `crates/filmcraft-h264enc/UPSTREAM.md`). Compression (2026-10-28): Speed now uses inter partitions, so desktop clips are about a third smaller at the same quality (still about 1.9x x264 `veryfast`'s bits; see `docs/performance.md`). Keyframes still stall 70-180 ms at 2 threads, 1080p60 software on 4 cores still needs a graphics-card encoder, and real game footage was not measured. FilmCraft's ffmpeg-oracle tests were not copied (they need its private test kit), so tuning work must add equivalent checks.

**Status:** active prototype · **Last reviewed:** 2026-10-25 · **Target:** local Windows capture-to-export alpha

The [current whole-workspace performance audit](performance-audit-2026-10-25.md) identifies a
confirmed idle keyboard-focus redraw loop, repeated library sorting/layout, synchronous UI index
writes and remaining decode/audio/transfer work. Its timings are cloud/generated diagnostics,
not gaming-PC budgets. M2.P2 now implements priorities 1–3 (below); the remaining proposals and
existing hardware gates remain open.

This is the ranked work list, not a parity claim. Capture, replay, playback, local library
organization, and no-reencode trim export are implemented. Linux/X11 has end-to-end synthetic
capture evidence; Windows/AMD has generated-window capture and native GPU encoding evidence,
including one-minute SDR 1080p60 measurements, with broader performance and hardware reliability
gates still open. Rank remaining work
by user impact, frequency, and evidence. Update this file in the same pull request whenever a
feature's status, workaround, or evidence changes.

| Rank | Gap | Evidence / current limitation | Impact | Work |
| ---: | --- | --- | --- | --- |
| G0 | Windows release and broader real-device evidence remain open | The Windows 0.2.0 setup program and separate CLI zip (thin-LTO release profile) build with `cargo xtask package`; the tag-triggered release workflow ran for v0.1.0. Windows/AMD generated-window WGC and native/CPU MF encode have run. Windows hotkeys, game-only arming, real microphone/speaker, device loss and other GPU encoders remain unverified. Linux/X11 capture-to-export is proven against synthetic sources | Limits platform and release claims | M0 evidence close-out; M2/M3/M5 |
| G1 | Low-copy GPU path is implemented for Windows SDR; broader performance gates remain open | WGC D3D11 leases and Media Foundation GPU NV12 conversion have one-minute AMD generated-window 1080p60 evidence with zero CPU readbacks and separate fresh/repeated-frame counters. SDR capture is now the default, and a different output size, crop or letterbox is scaled by the graphics card (D3D11 video processor, black bars) instead of forcing CPU readback; the scaled path has a Windows test but no real-hardware measurement. The optional HDR compatibility setting retains CPU readback. The earlier 32 fps wait stall did not reproduce before these changes and remains unexplained. A two-hour 1080p60 soak, other GPUs, HDR/scaling, GPU-use/power and real-game comparisons remain open; see [measurements](windows-performance.md). | Short generated-workload throughput is measured; game impact and broader reliability are not established | M5.1/M5.2/M5.2a |
| G2 | Game-only capture still needs real platform evidence | Recursive library scans and captured-game tagging are implemented and tested on Linux. Desktop scans are cancellable; manual labels, favorites and collections survive rescans. Game-managed saves snapshot the game at Save time. Real-game/Windows arming and Wayland process-to-window matching remain unverified or unavailable | The library workflow is usable, but automatic arming still needs desktop evidence | M4.1b/M7 |
| G3 | Compressed export needs reference-machine measurements; broader output controls remain open | Original-quality trim and software H.264 size presets (10 MB, 25 MB, custom 1–500 MB) are built. Generated MP4/MKV fixtures verify decoding, audio sync, cancellation, actual-size reporting, and collision safety. Quick-trim compression is at most 720p/30 fps, copies AAC audio, and estimates rather than guarantees size; 30/60-second throughput measurements on representative PCs remain open | Local small-file export is available; editor 720p/1080p/source (up to 4K) and 30/60 FPS are now built; broader codecs and speed claims remain limited | M4.2/M4.3/M11.1d |
| G4 | macOS capture/audio is not implemented; Wayland capture is only partly verified | No ScreenCaptureKit/CoreAudio provider; Wayland portal and PipeWire pieces pass component tests but no real desktop chooser has delivered a frame | Limits supported-platform claims | M6 |
| G5 | Real-device audio and broader performance evidence are incomplete | Windows/AMD passed a two-hour generated-window 1080p30 GPU soak: eight drops, zero restarts, all 24 saved clips decoded. Native SDR 1080p60 has one-minute generated-window measurements. Real audio devices, real-game impact, a two-hour 1080p60 soak and device-loss integration remain open; see [results](windows-performance.md) | Limits reliability and release claims | M3/M5 |

The core workflow exists and can be exercised locally on Linux/X11 with synthetic capture and audio.
Do not mark platform support from compilation alone; attach evidence for the user workflow and
required failure cases. Do not publish completion percentages until a repeatable scorecard exists.

Vice reports sub-1% CPU for its GPU Screen Recorder backend, but this is not a measured ClipCraft
result or a universal expectation. GPU Screen Recorder documents X11 and Wayland support across
NVIDIA, AMD, and Intel with driver, permission, and compositor caveats; ClipCraft uses it as a
behavior reference only because its GPL-3.0-only non-Rust/FFmpeg runtime is outside project rules.
M5.2 plans a Rust-native GPU-surface path and requires real per-device measurements before any
compositor, GPU-vendor, or CPU-use claim.

Settings › Games recognizes [348 bundled games](game-catalog.md) plus games discovered from local
Steam installation manifests. One background worker handles process refresh/matching and the
running-app picker, with one replaceable pending request and stale-result rejection. Local Steam
metadata refreshes about every 30 seconds or on **Refresh games**; processes refresh about two
seconds after each completed scan. No network catalog, account or service is used.
Custom rules win; global automatic-recognition off, built-in exclusions and Steam-ID exclusions
remain effective. Linux reads exact Steam process IDs; installed-directory path matching also
works without retaining full process paths. Default Windows Steam locations are searched; custom
Steam installation roots need an explicit CLI root and are not auto-found through the registry.
Corrupt individual manifests are skipped and counted; a failed library-list refresh retains the
previous catalog and shows a warning. Identification remains metadata-based and unverified for
real games, changed executables, protected processes or anti-cheat.

Automatic recording offers **Games only** (default), **Games, then desktop briefly** with a bounded
15–3,600 second timer, and **Desktop and games** for continuous desktop replay between games,
including startup. A game opening during the timer switches to its verified window on the next
successful process scan; its next exit starts a new timer. The top bar identifies desktop capture
and shows a countdown for temporary capture. Capture-open failures and scan errors never authorize
a temporary desktop fallback. Explicit Stop pauses the two desktop modes until Start; mode changes
and timer expiry are handled without changing source files. Automatic transitions in the desktop modes save the prior
replay (up to the configured replay length, not the entire session), with game labels only on game
captures. One save and one queued transition save are allowed; a full save queue pauses frame
production rather than growing without limit. Pending paths are kept distinct.

Generated-source tests cover timed/constant modes, reopening, timer reset/expiry, pause, failures,
source switching, saved-media validity and labels. Native X11 evidence and remaining platform checks are listed
in [testing.md](testing.md); Windows and real-game behavior remain unverified here. Automatic desktop
continuation requires an exposed monitor and cannot bypass Wayland's chooser or permissions.
The proposed leinstay/steamdb download remains unused; local manifests avoid that dependency.

Reference behavior from Vice now drives the engine's longest-match rule, user-managed game allowlist,
process observation, and automatic replay arming. The process watcher polls about every two seconds while
automatic switching is enabled; it stores executable basenames and exact Steam IDs, not command lines or paths.
The unused ClipRecorder timestamped audio-gap utility was removed in M11.1j; the active
clock-driven `AudioSource` mixer is unchanged. Upstream revisions, permission provenance, and
exact source files are recorded in [`upstream-ports.md`](upstream-ports.md).

## Local game data (M7.3)

Opt-in CS2/Dota 2 authenticated loopback GSI and a Windows/macOS League Live Client API adapter
are implemented, with native Games settings, config setup, live summaries/details and CLI commands.
All switches default off; capture is independent. Portable tests cover local-player selection,
privacy, malformed/oversized data, stale status, actual loopback requests, config/partial
preservation, HTTP framing and disabling/keyboard lifecycle. Windows compilation passes.
See [game integrations](game-integrations.md) and [ADR 0022](adr/0022-local-game-state.md).

Live CS2/Dota/League matches, target-OS League TLS and macOS runtime behavior are not verified here.
Linux League transport is explicitly unsupported. Snapshots are transient: clip annotations,
automatic highlights are not built. Discord publication now has a separate explicit-consent
policy ([M10.2](discord-presence.md)). No documented supported local
match-data interface was established for Overwatch/Deadlock; they retain process recognition.
This work closes no capture/platform release gate.

## Discord presence (M10.2)

Default-off basic activity and separate game-name/map-mode-score-hero consent are implemented in
General, using shared engine policy/settings commands and a bounded local IPC worker. Unix mock
tests cover READY/ack/ping, malformed/oversized data, timeout/rejection, coalescing/rate limits,
privacy clears, reconnect, ID change, late READY after opt-out and shutdown. UI/CLI disclose only
the permitted fields; capture is independent. See [setup and limits](discord-presence.md).

M10.2a adds Medal-inspired details/state, a local card preview
without an ID, and a shared build identity (`CLIPCRAFT_DISCORD_APPLICATION_ID`). Normal settings
need only a toggle; a custom public ID is an advanced developer override. The maintainer supplied
the shared public ID on 2026-10-24; it is now the default for ordinary builds and empty release
variables. The former missing-ID blocker and Windows empty-read/disconnect bug are fixed. A live
Windows client acknowledged basic activity using the configured identity. M10.2c removes the
typographic game badges: three fixed Discord CDN references provide game art, and the uploaded
CUT application icon supplies product art. No publisher image files or runtime HTTP lookup are
bundled. All three labelled game-art previews received real Windows acknowledgments.
Other-account audience checks, macOS runtime and live games remain
unverified. Mock success/compilation do not close these gates. Clearing is best effort when the
client/service is unreachable or the app
is killed; normal exit requests a bounded clear then disconnects. Saved rich clip metadata is
still separate future work.

M10.2a verification: 568 tests/34 targets pass (five existing ignored checks), including privacy,
identity fallback, bounded art, advanced keyboard Apply/reset and ID-independent preview. Native
Linux screenshots/private mock and generated-game capture confirm the game/product badge layout;
CLI previews and exporter no-overwrite checks pass. Windows/macOS compilation is not runtime proof.

## Playback delivery and remaining scheduling work (M11.1f)

The display-result queue now keeps at most one latest RGBA picture, one opening summary and one
terminal status. Polling returns at most three events, ordered opening → picture → terminal.
Seek, step and replay invalidate earlier pending/in-flight pictures and old end markers; failures
remain observable. Shutdown drops pending pixels. Picture dimensions are checked against
33,177,600 pixels (8K UHD) and an 8192-pixel edge before RGBA conversion.

This bounds pending results only: the UI can still hold a displayed/polled picture, the worker an
in-flight picture, and FilmCraft its decoder/GOP caches. It is not a total RSS budget. Control/audio
channels retain their existing separate behavior. M11.1g now coordinates worker-side decoding with
playback-first admission (below); a unified job queue/decoder pool and proxies are not built.
M11.1i adds optional GPU composition (below).
Decode/file calls remain non-interruptible. Verification: 577 workspace tests pass (five existing
ignored), including 69 player tests and prepared-editor/keyboard regressions; docs gate passes.
No export speed, GPU throughput or real-device claim is added. See [architecture](architecture.md), [performance](performance.md) and [testing](testing.md).

## Playback-first decode admission (M11.1g)

A shared coordinator now admits at most three worker-side stages, with at most one background
stage and 64 waiting callers (at most 48 background, reserving 16 registrations for foreground).
Queued playback/audio requests take priority over new background
admission. Player open/video, audio blocks, uncached library thumbnail generation, timeline source
open/pictures and waveform blocks/MP4 packets participate. Analysis releases its slot between
chunks. Stop, timeline cancellation and superseded video waits are checked every 20 ms while
waiting; an already-running codec/file call remains non-interruptible. Thumbnail's legacy API has
no cancellation parameter. Capacity/poison failures use existing failure reporting, not a crash.

These are logical slots, not reserved CPU cores, OS thread priority, a codec-thread budget, total
memory cap or measured speedup. Background work may wait under sustained foreground demand.
There is no unified worker pool or decoder cache; existing callers/queues still own their work.
Export/recording, metadata-only source-info and cached JPEG loading on the UI are outside this
coordinator. Proxies, physical-GPU composition throughput and actual game-contention evidence
remain open. Verification: 584 workspace tests pass (five existing ignored), including 76 player
cases and generated concurrent playback/thumbnail/waveform coverage. See
[ADR 0025](adr/0025-cooperative-decode-admission.md) and [testing](testing.md).

Native retesting of M11.1g found a percentage-seek shortcut ordering bug (M11.1h): digit keys
batched into a single UI frame were applied numerically, so the highest digit overrode the final
request. Playback now chooses the last pressed digit in event order. Popup/text-focus guards and
other transport shortcuts are unchanged. The failing-before/passing-after regression, native before/after checks and final 585-test gate
passed. After main's newer branding/performance merge, the combined build passed 602 tests/36
targets (five existing ignored) and repeated native playback/editor checks. Coordinator tests also
passed 50 repetitions and mixed-workload playback 10 repetitions. See
[testing](testing.md). This retest does not verify physical audio devices, GPUs or real games.

## Shared GPU composition (M11.1i)

Preview and sequence export now share aspect-fit scaling, the existing seven colour effects,
black/white fades and ordered FilmCraft text masks through a bounded wgpu/CPU compositor.
It preserves aspect ratio without upscaling, fixing the prior independent-axis export fit.
Preview renders on the decode worker; recipe changes invalidate superseded pictures and prepared
sources begin with their final look/text settings. UI handles remain interactive without painting
baked glyphs/fades twice. Library playback remains unchanged.

Missing/unsupported hardware, contention or failed GPU contexts use CPU. Automatic selection
rejects software adapters; a strict diagnostic used real llvmpipe Vulkan for 96 cases within one
8-bit channel level of CPU, plus real-context destruction/fallback. Local software timings include
upload/readback, not decoding or encoding. Physical GPU speed, Metal/DX12 runtime, real-game impact,
power and broader device-loss recovery remain unverified. Glyph rasterization, media decoding,
existing export YUV pre-reduction and RGBA conversion remain CPU work. This is not zero-copy.

Inputs remain capped at 8K UHD pixels/8192 edge, output at 4K UHD pixels/3840 edge, 256 masks,
2 Mi pixels per mask and 16 MiB combined masks. Reused GPU buffers retain their bounded high-water
capacities until release/restart; driver/codec/CPU copies are extra. Poll/map waits are bounded;
native driver calls are non-preemptible. No shader plugins, decoder pool, proxy or new sidecar.
The final workspace gate passes 610 tests (five existing ignored), strict Clippy and dependency
audits; native generated export/keyboard/screenshot checks preserve both source hashes.
See [ADR 0026](adr/0026-shared-gpu-compositor.md), [testing](testing.md) and
[performance](performance.md) for exact evidence and limits.

## Unused-code cleanup (M11.1j)

The desktop no longer paints a second fade/text/effect path or uploads a separate text texture
cache. Shared worker composition (including CPU fallback) remains authoritative. Only the selected
active text's key/canvas/bounds are cached for drag handles; a bounded raster is temporarily computed
when that geometry changes, then its pixels are dropped. Deselection/close clear this metadata.
The shared compositor retains its existing text/frame budgets; no total RSS or speed claim is made.

Removed uncalled game helpers, the unintegrated ClipRecorder audio-timeline utility and unused
local time wrappers. Exact-clock regression coverage now calls FilmCraft Tick directly. Sources,
settings/commands, active audio mixing, Windows bindings and failure-injection seams are preserved.
The old audio utility remains in Git history with its permission provenance recorded. Dependency
scanning reports no unreviewed unused crate; `windows-core` is an explicit macro-use false positive.
Full verification and native evidence are in [testing](testing.md).

## Idle and library responsiveness (M2.P2)

The static keyboard-focus redraw loop is fixed, with a failing-before/passing-after regression.
On the same software-rendered cloud development desktop, Tab-idle process CPU changed from
347.58% to 5.67% of one logical CPU in short observations; these are not gaming-PC budgets.
Main library normalized keys, sort/filter indices and sidebar counts are cached, cards use virtual
rows, and logical keyboard traversal reaches offscreen cards. The already virtualized editor
browser now caches search results. Mutations invalidate model data; probe facts stay current.

Scan persistence, metadata edits, probe batches and recorded-game registration run on one ordered
worker. At most 64 jobs/128 MiB accounted payload, including pending acknowledgements; at most two
48 MiB scan reservations. Engine index/queue limits still apply; loaded engine state, UI data,
transient copies/fallback and edit journal are additional bounded memory, not a total RSS cap.
Failed writes pause newer jobs for retry. Normal close waits; explicit failed-close discard or an
abrupt process exit can lose unacknowledged metadata. Source media and atomic index publication
are preserved. The 614-test gate, native scan/favorite/shutdown/keyboard screenshots and portable worker/race
checks are recorded in [testing](testing.md), with measurements in [performance](performance.md).
Startup index reads, cached JPEG loads, settings writes and some metadata-only source/file checks
remain synchronous. This does not add decoder pooling, proxies or hardware performance evidence.
See [ADR 0027](adr/0027-responsive-library.md).

## Connected workflow evidence (M5.3)

The current generated Linux/X11 session covers process/window recognition → F8 replay with virtual
system audio → game-labelled/favorited library and thumbnail → playback → two cuts and text → MP4
export → normal/crash restart. A portable Rust integration test carries the same recording across
capture/save, library, playback, editor, export and recovery, including non-silent decoded sound,
missing-source failure/reconnect, cancellation, collisions and retained metadata. The full gate
passes 535 tests across 33 targets. Windows, real games/anti-cheat and physical devices remain open.

This check found and fixed three safety/integration problems: editor import now uses the playback
reader's video duration instead of a longer AAC-padded container duration; copy naming skips occupied
partials/recovery files and broken links; original-quality trim exclusively owns its temporary and
publishes without replacing a destination created during export. Trims now need a hard-link-capable
filesystem like compressed/editor exports. Each regression failed before its fix. Existing older
drafts retain their saved bounds: re-add a full source or trim its end if it extends past playable
video. A killed export can leave a partial behind; retry picks a fresh name and preserves that file,
since it may belong to another live instance. Automatic reclamation is not implemented by this change.

Native source and exported sound fully decode and play through a private PulseAudio null sink via
the normal capture/playback adapters; no physical microphone or speaker is proved. Five app sessions
were audited with `strace`: no Internet-addressed connect/send calls were observed. Network namespace
creation was blocked by this sandbox, so this does not close the network-disabled release gate.
See [testing](testing.md) for exact steps, generated outputs, failures and limitations.

## Small sequence editor (M11.1a/M11.1b/M11.1c)

The editor uses FilmCraft's pinned track/cut algebra, media readers, H.264 and AAC codecs. Its
browser/preview/timeline workflow appends local references, splits at the playhead, trims source
ranges, reorders, duplicates and ripple-deletes segments. Linked source audio has per-segment
volume/mute and fades with the picture. Library/Effects/Text tabs expose black/white fade presets,
seven colour effects and independent text layers with timing, position, size, colour, fade,
split/duplicate and shared undo/redo. Dips do not overlap video segments or change edit duration.
Preview and export use the same FilmCraft text raster, canvas geometry and audio fade envelope.

The workspace shares the charcoal/lime palette and embedded Inter UI faces, with icon controls,
a ruler, a shortcut footer and resizable panels. Its own rail icon opens an independent browser;
Import clips and Send to editor add references, while right-click offers Details/Add to timeline.
Undo/redo retains at most 64 edits within an 8 MiB budget. Quick trim remains available.

Limits: one gapless video/linked-audio sequence, no separate segment-count cap, 30-minute total,
minimum segment 1/30 second and at most 256 text layers. Launch stays on Clips after draft recovery and the editor page scrolls in short windows (checked at 960×600 and 1440×900 under a virtual display). M11.1c adds internal current-state
recovery without a user-facing project format. It restores edits/text/source references/title/
playhead/preset paused, with fresh undo history. The 16 MiB record is debounced 500 ms; a normal
exit flush waits at most two seconds. Crash/stalled-filesystem timing can lose recent unsaved
input. Missing media stays referenced; damaged/unknown recovery is kept until explicit discard.
Export re-encodes SDR H.264/AAC MP4/MOV or
recorder-compatible MKV into H.264 MP4 at 30/60 FPS, with 720p/1080p or source framing up to 4K. Pictures fit with
black bars and no upscaling. Only the first source audio track is used (first two channels, mono
repeated), encoded as one 48 kHz stereo AAC track; muted/silent segments contribute silence.
Additional audio tracks, HDR, non-square pixels, detached audio, overlapping video,
cross-dissolves and arbitrary rendering controls remain unsupported. Editing/stepping uses the
30 FPS grid; exported cuts follow the selected output frame grid; audio follows integer timeline positions without per-cut encoder priming.

Export video offers standard output or estimated 10 MB, 25 MB and custom 1–500 MB targets. Targets
reserve AAC and container overhead before choosing video bitrate and may lower canvas resolution;
an impossible budget is refused. Actual size, target status, dimensions, selected FPS, target video bitrate and encoder are reported.
These are estimates, not guarantees of a service's upload limit. When enabled by the recording
encoder preference, sequence export tries the registered GPU encoder and restarts in software on
encoder failure. Sequence decoding stays in software; real-GPU export speed/quality remain
unverified. M11.1d adds 720p/1080p/source (up to 4K), 30/60 FPS and recovery of those
choices. Source canvases preserve the first clip's aspect ratio within 3840×2160 landscape or
2160×3840 portrait (8,294,400 pixels). Standard bitrate targets scale from 4 Mbps to at most
50 Mbps; file-size presets can reduce bitrate/resolution while retaining selected FPS. The
dialog explains reductions and shows the actual output plan. Codec selection and GIF/loudness
remain in their existing workflows. High-resolution text uses content-height masks with the
existing 2,097,152-pixel per-layer and 16 MiB active-cache budgets; overly large text returns
an actionable reduce/shorten error. M11.1d passes the 527-test/31-target full gate, generated
1080p/60 and 4K/text/portrait/ultrawide exports, hardware stand-ins and native Linux quality,
recovery, minimum-window and size-warning checks. A real native 1080p/60 export and a
1 MB-target 960×540/60 CLI export decode fully; source hashes are unchanged. See [ADR 0021](adr/0021-editor-export-quality.md).

M11.1e removes redundant thumbnail/canvas allocations for matching opaque output pictures and
per-block AAC channel-vector allocations. A generated two-second 1080p60 software export measured
12.128 s before and 7.504 s after on the Linux cloud VM, with byte-identical MP4 output; this is
not real-time export or a game/capture performance claim. Six-second exports measured 20.722 s;
the process high-water mark reached 575,100 KiB, including warm-up and allocator retention. Decoder
caches remain substantial; the removed 15.82 MiB/frame temporary allocations at 1080p are not a
total memory bound or a guaranteed process-peak saving. See [performance](performance.md) for the
machine, workloads, spread and reproduction commands. One worker still owns each export; latest
progress updates every frame, with progress wakeups spaced at least 100 ms apart and immediate
completion/error notification. Keyboard Space/stepping and mouse transport remain available while
edits/another export are blocked. Cancellation stays pending until acknowledged and cannot interrupt
a running codec/filesystem call. Progress/cancellation stay reachable in the top toolbar at 960×640;
other lower editor controls can still require a taller window. Development builds optimize export loops like the
player/codecs. Physical GPU, Windows/macOS and real-game interference measurements remain open.

M11.1c prepares the next source paused within a two-player active/prepared/retiring budget;
decoders that have not acknowledged cancellation retain their slots. Slow decoding/seeking can
still pause at a cut; real-time 4K playback and reference-machine export speed remain unmeasured.
The timeline now samples source pictures and first-track min/max audio envelopes rather
than repeating a library thumbnail and showing a placeholder audio lane. One analysis worker,
eight queued requests and a 32 MiB/256-entry result cache bound desktop work. Each result has at
most twelve 160×90 pictures and 2048 i16 min/max buckets; codec buffers are separate from these
payload bounds. MP4/MOV streams native-rate audio packets; other containers use an 8 kHz fallback
limited to ten seconds and 16 MiB nominal native PCM, otherwise analysis reports unsupported.
MP4 preflight caps movie/fragment metadata at 16 MiB, traversal at 100,000 boxes/eight levels,
declared sample entries at one million across the entire source, and each declared encoded sample
at 64 MiB before probing. Long PCM sources may exceed
that source limit even for a short requested range; oversized audio packets are refused before
decoding and oversized ALAC configuration before construction. Returned packet PCM is bounded
after decode; codec scratch allocations are additional. AAC recorder clips use far fewer sample entries.
Native video GOP/reference caches can be much larger than result caches; total process memory
and analysis latency need reference-machine measurements. The final software gate passes 520 tests
across 30 targets plus strict lints/layers/assets/licenses. Native 1440×900 checks show distinct
source strips, waveforms, keyboard actions and paused restart recovery of cuts/text/title/preset/
playhead with fresh history; missing-source placeholders preserve the edit. Corrupt recovery
remains byte-identical until explicit discard, discard does not resave on navigation, and normal
exit flushes a change made before the debounce. Zoom/pan screenshots verify source-time alignment
on the latest guarded build. All source hashes remain unchanged. The two shutdown-race fixes pass
focused regressions and the final full gate. M11.1c is software-complete for this slice, with
Linux virtual-display evidence only.
See [testing](testing.md).
Text rasters and caches have explicit memory
bounds rather than retaining every layer's rendered mask. Export preserves drafts/sources,
supports progress/cancel and uses exclusive temporary output with no-replace publication; the
destination filesystem should support hard links (otherwise a checked rename is used). The shared `editor.import`, `editor.edit`,
`editor.export`, `editor.timeline` and `editor.draft.*` commands accept bounded local JSON. See [editor.md](editor.md),
[testing.md](testing.md) and [ADR 0019](adr/0019-editor-preview-and-export-presets.md) for behavior,
evidence and remaining platform checks.

## Performance review (2026-10-22)

M2.P1 removes encoded-video payload copies from clip snapshots, caches replay GOP boundaries and
avoids redundant full-frame RGBA clears and unscaled interpolation tables. On a Ryzen 7 5700X3D
Windows CPU microbenchmark, a ~150 MB video snapshot fell from 68.070 ms to 0.034 ms and 1080p
RGBA conversion with four threads fell from 1.392 ms to 0.638 ms. These are synthetic CPU timings,
not game-FPS evidence. Windows SDR native surfaces avoid readback for compatible MF encoders.
Nonblocking latest-frame reads and a bounded SDR update-interval request have one-minute
[generated-window 1080p60 measurements](windows-performance.md), counting fresh images separately
from repeated output. The earlier 32 fps encoder wait stall stopped reproducing before these
changes; its cause remains unresolved. A two-hour 1080p60 soak and real-game impact remain open.
Hardware pending input is bounded, and drain errors are reported on stop without modifying saved clips.
CPU fallback frame clones, HDR cost,
worker churn, large-library traversal/index merging and real-hardware validation remain open.
See [the performance review](performance.md) for workloads, limitations and ranked follow-up work.

## Recording output options

- **Done:** capture, bounded replay, and MP4/MKV recording with configurable source, framing (stretch, black bars, or crop), resolution, frame rate, replay length, post-roll, and local storage limit are implemented. Settings are validated and saved locally (`recording.load_settings` / `recording.save_settings`, `clipcraft-cli recording settings`); reframe geometry is unit-tested and the Linux/X11 workflow is end-to-end tested.
- **WebM unavailable:** FilmCraft has VP9 and Opus decoders only. Pure-Rust encoders exist on crates.io (`rusty_vp9`, `opus-pure`, `rav1e`) but are young, carry many `unsafe` blocks, and have not been vetted. WebM stays greyed out until one passes a spike (decode check with ffprobe/ffmpeg) and a licence and `unsafe` review.
- **Export options:** original-quality trim copies samples to a new MP4. Small-file export can re-encode H.264/AAC MP4/MOV/MKV to H.264 MP4 with estimated 10 MB, 25 MB, or custom 1–500 MB targets. It caps output at 720p/30 fps, preserves AAC tracks, reports actual bytes, and supports progress/cancel. It does not add recording codecs or guarantee a service's upload limit.
- **H.265 (Windows, graphics card only):** the Video page's Codec setting records H.265 through the graphics card's Media Foundation HEVC encoder (AMD verified: `AMDh265Encoder`, Main profile, 8-bit 4:2:0, no B-frames). MP4 (`hvc1`/`hvcC`) and MKV are written; ffmpeg decodes the clips with no errors; on the same 1440p60 desktop and 10 Mbps target a clip was 5.2 MB against 9.4 MB for H.264. There is no software H.265 encoder: with the processor encoder, on a card without an H.265 encoder, or off Windows, ClipCraft records H.264 and the status says why. NVIDIA through NVENC and VA-API are still H.264 only (NVIDIA's Media Foundation encoder should work but is untested). In-app playback of H.265 decoded at the same speed as H.264 in a harness measurement (both below real time at 2560x1440).

### Recording settings (applied by M2 where noted below)

- Quality tiles (Low 360p24, Standard 720p60, High 1080p60, Custom with resolution, FPS, bitrate, encoder, codec).
- Global save-clip, long-recording, microphone-mute, push-to-talk, and screenshot hotkeys are registered while clipping; conflict checking and local persistence are implemented. Screenshot behavior is verified on Linux/X11 only.
- Audio: PC audio and microphone toggles, output and microphone device pickers, a live microphone level test, push to talk, mono microphone, separate tracks, volumes.
- Storage: capture folder, usage and free space, a storage limit, and a delete-oldest or stop policy. The limit applies only to ClipCraft's own clips; full-disk and cleanup paths have deterministic Linux tests.
- Capture: cursor, force window capture, HDR compatibility; aspect ratio (game, stretch, black bars, crop to 16:9) appears in its bottom **Advanced settings** section.
- Settings are rebuilt around rows (`apps/clipcraft/src/rows.rs`, pages in `settings/pages.rs`). An **Overview** page shows the current setup, live recording health (dropped frames, encode time), actionable suggestions (for example "use the graphics card") and three one-click profiles. Pages are grouped as Recording (Capture, Video, Audio, Replay, Games) and Settings (General, Storage, Hotkeys, About). Every row can show it differs from the default and be reset on its own; resolution, frame rate, bitrate, keyframe interval, replay length, post-roll, volumes, audio bitrate and the storage limit are sliders. The search box filters the actual rows in place, so a setting can be changed from the results, and links to the pages whose settings are lists or forms. Encoder, codec, effort, bitrate mode, file format, aspect ratio, sample rate and channels are always shown; the advanced toggle now only reveals custom-game forms.
- **Deleting clips:** a clip's right-click menu has Open, Send to editor, Show in the file manager, Copy path, Favorite, **Delete clip…** (red) and Add to collection; the clip page has a Delete clip button and the Delete key asks too. Every route asks first and then moves the file to the Recycle Bin (Windows shell delete with undo; `gio trash` on Linux; Finder on macOS), so it can be put back; nothing is deleted for good. The engine command `library.trash` (also `clipcraft-cli library trash <file>`) only accepts an existing regular video file, and the move is retried for two seconds in case the player still holds the file. Not done: restoring from inside ClipCraft, multi-select delete.
- **Saved-clip notification:** when a clip is saved a card slides in at the bottom right of the free part of the screen (its own always-on-top window that never takes the keyboard focus, so it shows over a borderless game and while ClipCraft is in the tray) with the clip's picture, its game, length, size and picture size, and buttons to Play (opens it in ClipCraft), Show in the folder and Delete (asks again on the card). It stays seven seconds, pauses while the pointer is on it, and can be switched off in Settings › Replay. Not done: exclusive-fullscreen games, which hide every other window; other monitors.
- General: launch at sign-in (Linux autostart entry, macOS launch agent, Windows Run key) and start minimized. On Windows a system-tray icon (left click opens; menu drawn by ClipCraft itself on right click, in the app's dark style: a status header, Open ClipCraft, Start/Stop recording, Save clip, Take screenshot, Open clips folder, Library, Settings, Quit ClipCraft; save and screenshot are enabled only while recording; folders open directly instead of being selected in their parent) and **Keep running in the tray** (default on) make the close button minimize instead of quit, so capture keeps running; the taskbar button stays because a hidden egui window stops receiving frames, which would stall capture. Hiding the taskbar button, and the tray on Linux/macOS, are not done. **Show advanced settings** is last, after Help; it appends labeled advanced controls to Games, Capture, Quality and Audio instead of adding a category. Custom-game add/edit controls use the same uniform card layout as every settings tab (verified on Linux/Xvfb only) with padded, responsive fields and appear only when advanced settings are on; hidden rules/encoder/audio values remain active. The local preference and 64-rule budget are unchanged. Keyboard disclosure/creation, category visibility, process-picker stale-ID recovery and layout/accessibility checks cover this behavior; see [settings.md](settings.md) and [testing.md](testing.md). Only the Linux startup path and native UI can be exercised in this container.

### Library organisation

- **Done:** favorites (heart on a card or in the clip view, right-click menu), collections (create, rename, delete from the sidebar; add or remove from a card's right-click menu or the clip view), and games (guessed from a sub-folder of the library or a recorder-style file name such as `Valorant 2024.03.01 - 21.04.mp4`; set by hand in the clip view). The sidebar lists All clips, Favorites, collections, and games with counts. All of it is saved in the local index, kept across rescans, and exposed as the `library.edit` command.
- **Recursive scans:** one root and up to eight child-folder levels, 20,000 total entries and 5,000 videos. Symlinks are skipped; distinct hard-link paths remain separate clips. The desktop uses one worker and one replaceable pending request. Cancel/replacement discards partial or stale results; filesystem calls already blocked on a drive cannot be interrupted. Unreadable children, depth/budget limits, and non-UTF-8/oversized paths fail the whole scan with an error; previous metadata is retained. Missing files remain offline entries.
- **Captured games:** game-managed saves snapshot the active game before post-roll and persist it in the index after a successful save. Manual labels take priority, then recorded labels, first child folder, and filename guesses. Invalid labels never prevent video saving; an index-write failure keeps the clip and reports a warning, with an in-memory fallback (bounded to 5,000 recent labels). Deleting the rebuildable index loses captured/manual metadata; no sidecar or media metadata is written. Manual desktop captures do not infer an active game.
- **Other limits:** a plain filename with no date or ` - ` is not treated as a game. Collections use paths; files moved outside ClipCraft need relinking. Multi-root browsing and automatic filesystem watching are not implemented.

### M2: capture and replay buffer

Verified here (Linux, a virtual X display, ffprobe and ffmpeg as outside judges): screen and
single-window capture with correct colours, window loss reported, global hotkeys with Caps Lock and
Num Lock, MP4 and MKV clips that ffprobe reads as H.264 at the right size and that ffmpeg decodes
without a complaint, framing (black bars, crop, stretch), size changes, a lost screen keeps what
was recorded, no overwriting, no partial files, a bounded buffer, storage-limit trimming that only
touches ClipCraft's own clips.

**Not verified or not built:**

- **Built-in title bar, screenshots, sound effects.** The window has no system frame: the top bar
  draws minimize, maximize/restore and close, drags the window, maximizes on a double click, and
  resizes from a thin band along each edge. Checked on Linux under a virtual display only (no
  window manager), so dragging, maximizing and edge resizing have **not** been seen working on any
  desktop. On Windows a window without a system frame may cover the taskbar when maximized and
  loses snap layouts; untested. Screenshots (F11 while clipping) save a full-size PNG next to the
  clips; verified with a real recording. Sound effects are generated tones (no sound files), played
  through the default output; heard by nobody here (no sound device), only their waveforms are
  tested. With "Record game and system sound" on, a cue may be picked up by the recording.
- **Game performance while clipping** (measured on a 4-core Linux machine, not on a gaming PC).
  The software encoder at 1080p60 used two full cores and still dropped 73 % of the frames, so on
  a real machine it competes with the game for processor time. What is done about it: the
  graphics-card encoder is the default (it falls back to the processor), the software encoder is
  capped at four threads, and on Windows the program runs a little below normal priority while
  clipping. **Not measured:** the effect on a game's frame rate on any machine. **Still costly:**
  HDR/scaled/incompatible paths still copy pictures from the graphics card to memory. The new
  native SDR WGC-to-Media-Foundation route avoids that readback on the tested AMD combination and
  has one-minute generated-window 1080p60 evidence. HDR compatibility remains enabled by default
  and uses CPU readback; this evidence requires it disabled. Real-game impact, a two-hour 1080p60
  soak and other GPUs remain unverified. See [the report](windows-performance.md).
- **Windows.** Generated-window Graphics Capture and AMD Media Foundation encode have run;
  Windows hotkeys, exclusive-full-screen games,
  the cursor option, protected content, and elevated windows are all untested.
- **macOS** has no provider (it needs ScreenCaptureKit).
- **Wayland** ([ADR 0012](adr/0012-wayland-capture.md), [docs/wayland.md](wayland.md)): the desktop
  portal and PipeWire are used. The portal conversation (against a stand-in desktop) and the PipeWire
  receiver (against a real PipeWire server with a real video node) are tested; **no real portal
  backend has delivered a picture** (xdg-desktop-portal-wlr refused on headless sway). GNOME and KDE
  choosers, window sharing, several monitors, and remembering the choice are untested or not built.
- **Sound** ([ADR 0009](adr/0009-audio-recording.md)). The microphone and the computer's sound
  are recorded as AAC next to the video, as one mixed track or two separate ones, with live level,
  mute (shortcut), push-to-talk, mono microphone, and recovery after a device is lost. Verified
  here with generated tones through the whole recorder, with ffprobe and ffmpeg as judges: tones
  at the right pitch and level on the right track in MP4 and MKV, mixing, mute and gain, sound
  length equal to picture length, a click lining up with its picture within 40 ms, a device clock
  1.5 % fast or slow staying in step, a device lost and recovered, bad layouts refused. The original
  end-to-end runs used generated sources. A Windows default-microphone adapter/denoiser smoke
  check now delivers finite samples, but real spoken clips, speaker listening and physical-device
  recovery remain open. Known limits: game/system sound is
  recorded by loopback on Windows and, on Linux, through the PulseAudio client library (also
  answered by PipeWire) from the monitor of the default output, verified here against a real
  PulseAudio server with a null sink (a tone played into it ends up in a saved clip); the output chosen in
  Settings › Audio (PulseAudio's outputs are listed by name and recorded through their monitors;
  verified with two null sinks: a tone played to one is heard only by its monitor) is what is
  recorded, and macOS has none; resampling is linear; per-application audio is not separated;
  screenshots are wired to a global hotkey and save PNGs beside clips; push-to-talk works through the global hotkey only (not when ClipCraft
  has the focus). Sound and picture of a clip can differ by up to a frame at the very end.
- **Settings not yet applied:** WebM.
- **Speed.** Measured on a shared 4-core container with a software-rendered video playing on the
  same screen, using the Fast preset: 360p30 keeps up (10 ms per picture); 720p30 keeps up (25 ms;
  241 of 241 pictures); 720p60 encodes about two of every three pictures (it drops a third); 1080p
  at any rate is not sustainable in software. Pixel conversion alone is 4 to 11 ms at 1080p. A
  real gaming computer is faster and the game also competes for it; hardware encoders are wired but
  need real GPU measurements. The top bar shows when more than 5% of pictures are dropped.
- **Crash and full-disk safety** ([ADR 0011](adr/0011-crash-and-disk-safety.md)). Clips, trims,
  settings and the library index are written to a temporary file and renamed, so an interruption
  never damages what exists. On launch ClipCraft removes its own unfinished `.partial` clips
  (and keeps a cut-off Matroska one that still plays) and says so. A damaged settings or index file
  is moved to `*.damaged` instead of being overwritten. An MP4 is first written as a fragmented
  crash-safe copy (`.mp4.recovery`, playable up to its last complete fragment) and then as the
  ordinary clip; after a crash the copy is turned back into a clip, with sound, up to the last
  whole fragment (one per keyframe interval). MP4 saves therefore write the clip twice and need
  twice the room while saving. A save refuses before writing when the disk
  cannot hold the buffer, and a disk that fills mid-save (tested on a real 2-16 MB tmpfs) reports
  "disk is full", leaves no partial file, keeps earlier clips, never deletes older clips to make
  room, and recording carries on. Not covered: the replay buffer itself lives in memory, so a
  crash loses what had not been saved; interruptions were simulated with cut-off files, not by
  killing the app mid-write; the disk tests are skipped where mounting is not allowed.
- **Soak, suspend, and diagnostics** (see [release checklist](release-checklist.md)). A freeze of
  10 s or more in the pictures (the computer slept) starts the replay buffer over rather than
  leaving a frozen stretch in clips; `gap_restart_seconds` sets it. The soak tool measured
  720p30 + sound at +21 to +26 MB memory after warm-up over 3 to 4 minutes. Windows/AMD also passed
  a two-hour generated-window 1080p30 GPU soak: eight drops, zero restarts, 177.77 MiB peak RSS and
  24 saved clips decoded. Native SDR 1080p60 also has one-minute generated-window measurements;
  a two-hour 1080p60 soak and the broader release gates remain open. See
  [details](windows-performance.md).
  Tab moves a visible focus ring through every control and Enter activates; the
   Tab starts at the top bar, then the navigation rail, then the page.
  `clipcraft-cli diagnose` and Settings › General › Copy report list folders, screens, sound, and
  encoders. Screen-reader names are tested through the accessibility tree, not with a real screen
  reader.
- **Missing files.** A clip whose file is gone is listed as "File not found" with Remove from library
  (card footer, right-click menu, or the clip view when it fails to open). Find file asks for a
  folder and searches it and the folders inside it (8 deep, links not followed, bounded) for the same
  name and size, then relinks the clip, keeping its favorite, game and collections; a relinked clip
  outside the scanned folder stays online. The folder-choosing dialog could not be driven in the
  headless test display, so only the search, relink, and rescan are tested, not the dialog. Removal only forgets the
  library entry, refuses while the file still exists, and survives rescans and restarts.
- **Trim and export** ([ADR 0010](adr/0010-trim-export.md)). Any clip in the library can be cut to a
  range and saved as a new MP4 without re-encoding: from the clip view (scissors, T, I, O) or
  `clipcraft-cli clip trim IN OUT START END`. Verified with ffprobe and ffmpeg: the first picture is
  bit-identical to the source picture at the start time, length is right, sound lands within 40 ms,
  for clips from ClipCraft's own MP4 and MKV and for an ffmpeg file with B-frames; cancelled, bad,
  and existing-output requests leave nothing behind and the input byte-identical. Limits: players
  that ignore MP4 edit lists show up to one keyframe interval (2 s by default) of extra picture
  before the start; Matroska input is H.264 + AAC without B-frames only; output is MP4 only; there
  is no re-encode, so the cut cannot change size, speed, or format; AVI and WMV files are playable
  but not probed or cut.
- **Post-roll.** "Keep recording after the hotkey" (off, 3, 5, 10, 15 s) delays the save and adds the
  recorded seconds to the clip; the top bar counts down. Stopping the recording during the wait
  saves at once with what exists. A second press during the wait is ignored. Tested with the
  synthetic screen (engine tests); not yet run against a real game.
- **Clipping keeps no full recording.** Only the last replay seconds (plus post-roll) live in memory;
  disk is written only when a clip is saved. The Video tab says so and shows an estimated size per
  clip and per minute (video plus sound bitrate, +2 % container). Variable bitrate can come out
  smaller; the estimate is the target, not a measurement of real clips.
- **Clip length** can be up to one keyframe interval longer than the replay setting, and a clip
  from a struggling computer is variable frame rate.
- **Saved clips are named in UTC** (`ClipCraft 2026-10-19 21-04-55.mp4`).
- **Changing a quality setting while recording** needs a restart; the Settings page offers it.
- **A resize** restarts the stream and drops the buffered video.

### M4.4: GIF and loudness (implemented; generated-clip evidence only)

The trim panel's output menu now also offers **Animated GIF** (up to 30 s) and **Even out
loudness** (-16 LUFS), sharing `clip.export_gif` and `clip.normalize` with the CLI. Evidence is
Rust-generated clips decoded back (and checked with FFmpeg when present); nothing was run on
Windows, macOS or with real games, and the UI was only exercised through its headless draw tests
and a manual Linux run. Joining clips (and transitions) is deliberately left for the planned
editor. Small-file export also tries the GPU encoder (same choice as recording) and falls back to
software on any trouble; that path is tested with a stand-in encoder only, and an upper-bound test
shows decoding the source limits it to about a third faster. FilmCraft's OS hardware decoders are
now registered (off for playback, on for GPU-allowed exports) to lift that limit; untested on a
real GPU. Small-file export was also made much faster (see [testing.md](testing.md)); the numbers are
from a 4-core Linux container, not a gaming PC. See
[ADR 0018](adr/0018-gif-and-loudness-export.md).

### M4.3: local export size presets (implemented, broader controls open)

The player trim panel and CLI `clip export INPUT OUTPUT START END TARGET_MB` now offer local
small-file H.264 export. The original-quality copy path stays available. Size targets are decimal
MB, software encoding uses two threads, and output is capped at 1280×720 and 30 fps without
upscaling. Lower budgets select smaller dimensions. AAC tracks and their edit offsets are copied;
if audio leaves insufficient room for video, the job asks for a larger target or shorter range.

Rust-generated 60 fps MP4/MKV fixtures verify 30 fps output, duration, the first picture of a cut
between keyframes, two audio tracks with a click aligned within 50 ms, monotonic progress, source
preservation, cancellation cleanup, malformed input, temporary-file collisions, and a destination
created while encoding. FFmpeg additionally verifies decoding when available. No GPU is needed.

Limits: H.264/AAC sources only, at most 16 audio tracks, at most 30 minutes per export. Explicit
HDR and non-square-pixel sources are refused; HDR mastering and arbitrary metadata are not
preserved. Output uses a hard link for atomic no-replace publication, falling back to a checked rename on filesystems without hard links.
One-pass size targets can be exceeded, and the completion report says so. Audio bitrate reduction,
user-selected resolution/FPS/codecs, and representative 30/60-second performance measurements
remain open. No upload, Discord account, link, or network path is added. See
[ADR 0013](adr/0013-local-compressed-export.md).

### Force window capture, HDR, and GPU encoding

- **Force window capture** records the window with the focus (found through `_NET_ACTIVE_WINDOW`
  on X11, `GetForegroundWindow` on Windows) instead of a screen, unless a game window was chosen.
  ClipCraft's own window is never picked. Verified on X11 with a published focus hint; on Windows
  it is compiled only. Because clicking Record puts ClipCraft in front, it only finds the game
  when recording is started with its hotkey while the game has the focus; the app says so.
- **HDR compatibility** tone maps linear scRGB half floats (Windows' HDR format) to standard
  range: reference white 203 nits, a smooth highlight roll-off, sRGB encoding. The maths is unit
  tested. Windows capture asks for half floats when the setting is on. **Not run on an HDR
  screen**, and it guesses "this frame is HDR" from any value above 1.05 instead of asking
  Windows for the screen's SDR white level. HDR content in BT.2020 outside BT.709 is clipped to
  BT.709. X11 reports no HDR.
- **GPU encoding** uses FilmCraft's NVENC encoder (Windows and 64-bit Linux, NVIDIA only) or
  VideoToolbox (macOS), asked for H.264 Baseline so pictures stay in order. The recorder is tested
  against a stand-in hardware encoder that produces real H.264: a valid clip, the fitted picture,
  a missing GPU (software, with the reason in the status), a GPU that fails mid-recording or
  reorders pictures (handed to the software encoder and the buffer restarted). **No real GPU
  encoder has run**: this container has none, so NVENC and VideoToolbox are untested. AMD and
  Intel graphics are reached through Windows Media Foundation instead (next item). H.265 through the GPU
  is wired for Media Foundation only (see the H.265 item). The Video tab probes the card once at start-up
  and greys out GPU when it cannot be used.
- **AMD and Intel on Windows** record in hardware through a Media Foundation H.264 encoder
  ([ADR 0007](adr/0007-media-foundation-encoder.md)), tried after NVIDIA's. The stream handling
  (Annex B to length-prefixed, the decoder configuration record, RGBA to NV12) is unit tested on
  Linux; the Windows code compiles but has **never run on a Windows computer or an AMD or Intel
  encoder**. If a driver will not take the request, recording uses the CPU and the status says
  why.
- **AMD and Intel on Linux** record in hardware through VA-API
  ([ADR 0008](adr/0008-vaapi-encoder.md)), tried after NVIDIA's. libva is loaded at run time, so a
  computer without it just uses the CPU. Verified here: every libva structure's size and field
  offsets against the real C headers, the SPS and PPS the code writes against FilmCraft's parser,
  and the encoder's call order, parameter buffers, pixel upload, key-frame cadence, and clean-up
  after a failure at each step against a stand-in libva. **Never run against a real Mesa or
  Intel driver**: this container has no GPU. Driver differences (packed headers, slice headers,
  entry points) are the likely first failures; the status line names the reason and recording
  continues on the CPU.
