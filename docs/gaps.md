# Known gaps

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

**Status:** active prototype · **Last reviewed:** 2026-10-24 · **Target:** local Windows capture-to-export alpha

This is the ranked work list, not a parity claim. Capture, replay, playback, local library
organization, and no-reencode trim export are implemented. Linux/X11 has end-to-end synthetic
capture evidence; Windows/AMD has generated-window capture and native GPU encoding evidence,
including one-minute SDR 1080p60 measurements, with broader performance and hardware reliability
gates still open. Rank remaining work
by user impact, frequency, and evidence. Update this file in the same pull request whenever a
feature's status, workaround, or evidence changes.

| Rank | Gap | Evidence / current limitation | Impact | Work |
| ---: | --- | --- | --- | --- |
| G0 | Windows release and broader real-device evidence remain open | Windows/AMD generated-window WGC and native/CPU MF encode have run. Windows hotkeys, game-only arming, real microphone/speaker, device loss and other GPU encoders remain unverified. Linux/X11 capture-to-export is proven against synthetic sources | Limits platform and release claims | M0 evidence close-out; M2/M3/M5 |
| G1 | Low-copy GPU path is implemented for Windows SDR; broader performance gates remain open | WGC D3D11 leases and Media Foundation GPU NV12 conversion have one-minute AMD generated-window 1080p60 evidence with zero CPU readbacks and separate fresh/repeated-frame counters. Matching dimensions/original framing and HDR compatibility disabled are required; the default HDR-compatible setting retains CPU readback. The earlier 32 fps wait stall did not reproduce before these changes and remains unexplained. A two-hour 1080p60 soak, other GPUs, HDR/scaling, GPU-use/power and real-game comparisons remain open; see [measurements](windows-performance.md). | Short generated-workload throughput is measured; game impact and broader reliability are not established | M5.1/M5.2/M5.2a |
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
ClipRecorder's timestamped audio-gap policy is also ported in `clipcraft-record`, but the current
untimestamped `AudioSource` mixer does not use it. Upstream revisions, permission provenance, and
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
minimum segment 1/30 second and at most 256 text layers. M11.1c adds internal current-state
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
destination filesystem must support hard links. The shared `editor.import`, `editor.edit`,
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
- **H.265:** recording is H.264-only. FilmCraft supplies a pure-Rust HEVC decoder; no H.265 encoder path is connected to ClipCraft's file writer, so H.265 remains unavailable.

### Recording settings (applied by M2 where noted below)

- Quality tiles (Low 360p24, Standard 720p60, High 1080p60, Custom with resolution, FPS, bitrate, encoder, codec).
- Global save-clip, long-recording, microphone-mute, push-to-talk, and screenshot hotkeys are registered while clipping; conflict checking and local persistence are implemented. Screenshot behavior is verified on Linux/X11 only.
- Audio: PC audio and microphone toggles, output and microphone device pickers, a live microphone level test, push to talk, mono microphone, separate tracks, volumes.
- Storage: capture folder, usage and free space, a storage limit, and a delete-oldest or stop policy. The limit applies only to ClipCraft's own clips; full-disk and cleanup paths have deterministic Linux tests.
- Capture: cursor, force window capture, HDR compatibility; aspect ratio (game, stretch, black bars, crop to 16:9) appears in its bottom **Advanced settings** section.
- General: launch at sign-in (Linux autostart entry, macOS launch agent, Windows Run key) and start minimized. **Show advanced settings** is last, after Help; it appends labeled advanced controls to Games, Capture, Quality and Audio instead of adding a category. Custom-game add/edit controls use the same uniform card layout as every settings tab (verified on Linux/Xvfb only) with padded, responsive fields and appear only when advanced settings are on; hidden rules/encoder/audio values remain active. The local preference and 64-rule budget are unchanged. Keyboard disclosure/creation, category visibility, process-picker stale-ID recovery and layout/accessibility checks cover this behavior; see [settings.md](settings.md) and [testing.md](testing.md). Only the Linux startup path and native UI can be exercised in this container.

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
  1.5 % fast or slow staying in step, a device lost and recovered, bad layouts refused. **Never run
  with a real microphone or speaker** (this container has no sound card), so the device layer
  (`DeviceCapture`, cpal) is untested beyond its queue. Known limits: game/system sound is
  recorded by loopback on Windows and, on Linux, through the PulseAudio client library (also
  answered by PipeWire) from the monitor of the default output, verified here against a real
  PulseAudio server with a null sink (a tone played into it ends up in a saved clip); the output chosen in
  Settings › Audio (PulseAudio's outputs are listed by name and recorded through their monitors;
  verified with two null sinks: a tone played to one is heard only by its monitor) is what is
  recorded, and macOS has none; resampling is linear; per-application audio is not separated;
  screenshots are wired to a global hotkey and save PNGs beside clips; push-to-talk works through the global hotkey only (not when ClipCraft
  has the focus). Sound and picture of a clip can differ by up to a frame at the very end.
- **Settings not yet applied:** H.265 and WebM.
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
  disk is written only when a clip is saved. The Quality tab says so and shows an estimated size per
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
preserved. Output needs a filesystem supporting hard links for atomic no-replace publication.
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
  is not wired (the file writer is H.264 only). The Quality tab probes the card once at start-up
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
