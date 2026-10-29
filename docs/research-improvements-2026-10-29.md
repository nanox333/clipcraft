# ClipCraft improvement research — 2026-10-29

Reviewed fetched `main` at **f91de58** (0.2.0), the active implementation, feature/gap/backlog and
performance reports, and public documentation from Vice, ClipRecorder, GPU Screen Recorder and the
platforms below. This is a research and prioritization report, not a claim that proposals are built,
benchmarked or approved for release. The product remains Rust-first and local-first: no required
server, account, telemetry, social feed, sharing service or non-Rust runtime. Optional update checks
already exist and are enabled by default; they can be disabled. Optional encrypted backup is a
separate, explicitly configured future feature.

## Already in the current build

Game-only arming and desktop fallback modes; configurable replay length/post-roll and multiple
bindings per shortcut; mixed/separate system and mic audio; offline suppression and mic test;
Windows hardware H.264/H.265, fallback and retry; a vendored Rust H.264 software encoder; X11
shared-memory capture and Linux portal/PipeWire pieces; a local library with filters and per-game
folders, trash, playback, GIF/trim/size-target export; a small sequence editor; local match-data
adapters and optional Discord presence; first-run setup, Windows tray, installer and update check.
Playback already has a six-frame/96 MiB decode-ahead queue and optimized 8-bit RGBA conversion.
The Windows SDR capture-to-Media-Foundation path and library idle/sort/index-worker improvements
are already built; do not schedule them again. Encoder measurements in `performance.md` include
real improvements but not high-motion gameplay evidence. Automatic game/desktop transitions do
**not** save a clip; they replace the old replay. Hardware-encoder recovery also clears the replay
when switching back to the GPU (`try_return_to_hardware` in `recorder.rs`).

## Recommended order

| Rank | Improvement | Why / current evidence | Next concrete slice and acceptance |
| ---: | --- | --- | --- |
| 1 | **Make replay readiness and gaps unmistakable** | When capture changes source or the encoder switches, the old replay can disappear; a user can press the hotkey before a full buffer has refilled. Automatic game exit does not save it. The fallback/retry path explicitly clears the ring | Show current buffered duration and source/encoder transition as an in-app status and non-focus-stealing toast; keep the existing explicit save policy. Fake-provider tests for game exit/reopen, repeated failure/recovery, a save at every transition and actual clip duration; never invent or overwrite a clip. |
| 2 | **Wayland portal hotkeys** | Current evdev hotkeys require access to raw keyboard events, an important permission/privacy tradeoff. The [GlobalShortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) specifies bound actions and Activated/Deactivated signals | Add a version/capability-probed portal adapter for save/screenshot/PTT where the desktop supports it. Keep explicit consent and existing configurable fallback; do not silently read keyboard devices. Test portal denial, collision, reconnect, PTT release, multi-binding, fake server and real GNOME/KDE/wlroots versions separately. Portal availability and key choices vary by desktop. |
| 3 | **Prove real capture end-to-end and expose useful diagnostics** | Windows/AMD and Linux/X11 have generated-source evidence; real games, other GPUs, Wayland desktops, real audio, device loss and offline release flow remain gaps | Prepare a reproducible matrix and privacy-preserving diagnostic snapshot: active backend/codec, frame dimensions/rate, actual buffered seconds, fresh/repeated/dropped frames, audio track status and last failure without file paths/content. Automate generated sources and failure injection now; separately run named hardware/game cases when available. Compare game frame times, not just recorder CPU. |
| 4 | **Linux portal/PipeWire correctness and low-copy research** | No real GNOME/KDE/wlroots portal chooser has produced a verified frame; current PipeWire reader expects mapped data and copies packed frames. Linux hardware paths still need real GPUs | First prove portal permission, restore-token rotation, sizes/stride, fractional scaling, multi-monitor, unplug/reconnect and mapped buffers. Then investigate DMA-BUF negotiation, device/format compatibility and GPU encoding through narrow Rust platform interfaces with bounded ownership and CPU fallback. No universal zero-copy or vendor claim without measurement. |
| 5 | **Harden release/update verification** | The Windows installer and opt-in install handoff exist; updates are checked by default. Downloads are unsigned; checksums verify against the same release and do not protect against a compromised publisher. Full upgrade from a release containing the installer is not yet evidenced | Test fresh install, update, rollback, uninstall and offline startup on a clean second Windows user/machine. Add signed release verification/code signing when maintainers have signing material; test wrong hash, malformed/oversized responses, interrupted downloads and retained clips/settings. Show update-network setting clearly. |
| 6 | **Lower playback and preview CPU without losing scrubbing quality** | Windows harness showed ~250% of one core playing 1440p60 after improvements. A two-thread decoder reduced CPU but increased median seeks (304 ms vs 212 ms); hardware decoding was not better on that machine | Prototype output-size-aware decode/native-plane reduction and reuse of one source across adjacent cuts. Compare real preview/export framing, colour/text, seeks and A/V sync against full-resolution references. Keep bounded queues and the high-quality fullscreen/export path. Evaluate caching/proxies only after measuring. |
| 7 | **Remove encoder stalls and improve actual bitrate control** | Recent 1080p desktop work improves size but can increase Speed processing time; keyframes still stall and a 12 Mb/s target produced ~9 Mb/s in one measurement. High-motion gameplay unmeasured | Benchmark vendored encoder on deterministic static, scrolling and high-motion generated/gameplay fixtures; profile I-frame tail latency, rate-control overshoot/undershoot and multi-thread scaling. Preserve valid output, frame timing and independent decode/quality checks. Quality/size/speed are a tradeoff, not a universal percentage. |
| 8 | **Finish export-size and clip usability** | Existing 10/25/custom MB options estimate target, may miss it, and quick compressed export caps at 720p/30 | Give clear expected/actual size, per-user target presets and an optional bounded re-encode/retry when a strict size is requested. Preserve sources, cancellation, audio sync and atomic publication. Do not hard-code Discord's changing limits as a guarantee. |
| 9 | **Independent clip-duration presets** | ClipCraft already permits multiple key bindings per action; Vice documents individual keys with distinct clip durations. These are different capabilities | Add a small bounded list such as 30 s/2 min/5 min, each with its own optional hotkey and actual-buffer availability feedback. Use the existing typed command/settings path, handle collisions, partial buffers and concurrent saves. No presets need to be enabled by default. |
| 10 | **Safer Linux hotkey onboarding** | Until a portal is implemented/available, evdev access means the user or other processes with device permission can read keyboard events; `docs/linux.md` already explains this | Present a clear permission/status and exact fallback in setup/diagnostics. Keep hotkey processing ephemeral and avoid logging raw keys. Test multiple keyboards, hot-unplug, access revocation and PTT release. Do not imply a udev rule isolates key access to ClipCraft. |
| 11 | **Microphone quality and device recovery** | Local noise suppression and mic listening are implemented, with generated PCM and a short adapter run; real speech, unplug/replug, clock drift and audible latency still need proof | Exercise speech + game audio, mono/stereo, PTT, suppression toggles, mixed and separate tracks, device changes, long drift and recovery. If needed, reduce callback lock contention and preallocate buffers; test overrun behavior before any lock-free redesign. Avoid echo-cancellation promises without a real reference signal. |
| 12 | **Simple editor finish** | Cuts, trim, reorder, text, fades, seven effects and presets already exist; user wants clipping first and a basic editor | Prioritize responsive seek, exact cuts, undo/recovery, reliable audio sync, visual trim handles and keyboard shortcuts. Reuse one decoder for adjacent same-source cuts and avoid reopening sources on each validation/render pass, within memory limits. Do not turn the MVP into a full NLE. |
| 13 | **Local library conveniences** | Cached/virtualized main grid, local game folders, search, favorites, collections and trash exist. Cached JPEG loading and startup index reads can still touch disk on UI paths | Move thumbnail cache reads/decoding to bounded cancellable workers with visible-first priority; retain labels/favorites on rename/relink, filters and interrupted scans. Add safe rename and keyboard-first actions if not already available after confirming UI. Test slow/removable disks, 5k/10k entries, stale results and no source mutation. |
| 14 | **Per-game profiles and hotkey conflicts** | Game recognition/custom exclusions exist, but different games may need different capture settings or may own the same key | Design a small local per-game override for resolution/FPS/audio/optional clip durations and optional hotkey suppression when a chosen game is foreground. Clear global defaults/override status, safe switching and backoff, no process injection. Test overlaps, game exit, manual capture and inaccessible/protected windows. |
| 15 | **Idle/background footprint** | M2.P2 fixed the Tab repaint loop and cached the main library. Process watching, status timers, tray, thumbnail work and encoder retry still run under their respective conditions | Measure idle, minimized, game-not-running, recording, playback, save and export separately on Windows/Linux. Coalesce redundant wakes and prioritize foreground work only where measured. Preserve hotkeys and game-start response; track CPU, GPU, memory, disk and battery, not just one metric. |
| 16 | **Encrypted backup research** | Backup is not built. User wants encrypted cloud backups without a ClipCraft server/account | Evaluate [`rustic_core`](https://github.com/rustic-rs/rustic_core) as a pinned Rust library candidate (its core manifest declares MIT OR Apache-2.0), or another reviewed library; no dependency is approved by this research. Define user-selected provider, locally held key, metadata leakage, recovery/lost-key behavior, authentication, conflict/delete/restore, resumability, integrity and offline-first behavior. The [restic format](https://restic.readthedocs.io/en/stable/100_references.html) is a public reference, not permission to bundle a non-Rust sidecar. Prove round-trip restore with a local fake provider and independent crypto review before enabling network transfer. |
| 17 | **macOS feasibility, then native provider** | macOS capture/audio is not implemented; compiling a shared crate is not runtime evidence. Apple provides [ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit) for screen capture | Prototype consent and window/display capture behind the platform layer, then evaluate audio, hotkeys, VideoToolbox, app lifecycle and packaging. Test screen-recording permissions/revocation and device changes on real macOS before advertising support. Respect OS indicators and protected content. |
| 18 | **Controller controls and input overlays** | Both are planned, not current features. They could help couch/controller users, but event capture and anti-cheat/privacy behavior vary by OS | Start with opt-in controller save-clip binding and a clear active state. Overlay input only after explicit consent, event minimization, visible indicator, no sensitive key logging, and desktop/game compatibility research. Avoid process injection and anti-cheat workarounds. |
| 19 | **MCP/CLI and extension safety** | The local CLI already calls engine commands; MCP/plugins are planned. Arbitrary plugins create significant safety, stability and maintenance costs | Add machine-readable bounded CLI operations first; consider MCP over local stdio using the same engine commands and configured paths. Design read/write permissions, validation and resource limits before any plugin or user shader runtime. No background AI uploads or server required. |
| 20 | **Formats and compatibility** | H.264 MP4 is the main export; H.265 hardware capture is Windows-specific; WebM recording disabled; upload-sized presets are estimates | Build a compatibility matrix for H.264/H.265/AV1 and MP4/MKV/WebM, actual target GPU/API licensing and players. Prioritize a format only after hardware, size, quality and playback tests; preserve fallback rather than silently selecting an unsupported codec. |

## Useful product ideas from public references

Vice documents separate clip-length hotkeys, a match recording with in-session markers, follow-mouse
monitor capture, automatic game playlists, hotkey suppression for selected games, and an opt-in input
overlay. These are behavior references, not commitments. ClipCraft already has long recording,
per-game folders, collections and rebinding; session markers and independent duration hotkeys are
separate potential additions. Follow-mouse and overlay behavior need per-compositor/OS validation.
Neither public links/tunnels nor a community feed fit the local product scope. Vice's stated CPU
and compositor claims are its own and cannot establish ClipCraft results.

ClipRecorder describes a small Windows ring-buffer recorder and tray; ClipCraft already has both
concepts. Its reported CPU figure is not a benchmark for ClipCraft. GPU Screen Recorder documents
GPU-native recording, driver-specific limitations, optional KMS privileges and suspend/VRR caveats;
it is GPL-3.0-only and uses a non-Rust/FFmpeg stack. It is a behavior/API reference, not a dependency
or source port under this repository's current policy. Real GPU work should use public API specs,
reviewed permissive dependencies and measured fallbacks. Claims about a single vendor or low CPU
cannot be generalized across drivers, graphics devices or desktops.

## Evidence and sources

- Current project: [feature list](features.md), [gaps](gaps.md), [roadmap](../ROADMAP.md),
  [backlog](backlog.md), [performance](performance.md), [Windows measurements](windows-performance.md),
  [Linux limits](linux.md), [update decision](adr/0032-optional-update-check.md),
  [encoder provenance](../crates/filmcraft-h264enc/UPSTREAM.md).
- [Vice README at `7e4b2a3`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/README.md),
  and [ClipRecorder README at `153025b`](https://github.com/By183/ClipRecorder/blob/153025b4c88ed42a23a97e030ce140c2fc54949d/README.md),
  accessed 2026-10-29. Reported personal permission for earlier selected ports does not establish
  permission or license terms for unrelated source/assets; provenance is in [upstream-ports](upstream-ports.md).
- [GPU Screen Recorder README](https://git.dec05eba.com/gpu-screen-recorder/plain/README.md), accessed
  2026-10-29; project and README may change. No source from it is used by this research.
- [XDG GlobalShortcuts](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html)
  and [ScreenCast](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html).
  The latter describes single-use restore-token rotation and, for interface version 6, prefers the
  PipeWire serial over the deprecated stream node ID for targeting; applicability depends on portal versions.
- [Windows screen capture](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture)
  and [Apple ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit).
- [rustic core manifest](https://github.com/rustic-rs/rustic_core/blob/main/crates/core/Cargo.toml)
  and [restic repository format](https://restic.readthedocs.io/en/stable/100_references.html).
  License strings alone do not establish dependency, API, unsafe-boundary or cryptography approval.

No new physical-device measurements were performed by this research. Existing measurements
and release gates remain attributed to their original reports. New implementation requires its
own issue with limits, failure cases, correctness and performance evidence, documentation and CI.
