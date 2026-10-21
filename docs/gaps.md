# Known gaps

**Status:** active prototype · **Last reviewed:** 2026-10-21 · **Target:** local Windows capture-to-export alpha

This is the ranked work list, not a parity claim. Capture, replay, playback, local library
organization, and no-reencode trim export are implemented. Linux/X11 has end-to-end synthetic
capture evidence; Windows/AMD now has generated-window capture and native GPU encoding evidence,
with unresolved 1080p60 throughput and hardware reliability gates. Rank remaining work
by user impact, frequency, and evidence. Update this file in the same pull request whenever a
feature's status, workaround, or evidence changes.

| Rank | Gap | Evidence / current limitation | Impact | Work |
| ---: | --- | --- | --- | --- |
| G0 | Windows release and broader real-device evidence remain open | Windows/AMD generated-window WGC and native/CPU MF encode have run. Windows hotkeys, game-only arming, real microphone/speaker, device loss and other GPU encoders remain unverified. Linux/X11 capture-to-export is proven against synthetic sources | Limits platform and release claims | M0 evidence close-out; M2/M3/M5 |
| G1 | Low-copy GPU path is implemented for Windows SDR; performance gates remain open | WGC D3D11 leases and Media Foundation GPU NV12 conversion produced AMD generated-window MP4 clips with zero CPU readbacks. Matching dimensions/original framing and HDR compatibility disabled are required. Both CPU/native runs encoded about 32 fps at requested 1080p60. Other GPUs, HDR/scaling, GPU-use/power and real-game comparisons remain open; see [measurements](windows-performance.md). | 1080p60 throughput and game impact are not established | M5.1/M5.2 |
| G2 | Game-only capture needs real platform evidence; library scan is non-recursive | Game-only mode, local process matching, exact Steam app IDs on Linux, manual pause, and X11/Windows process-to-window lookup are implemented. The X11 adapter has a synthetic-window test; no real game or Windows runtime has been tested. Wayland game-window matching is unavailable; library labels are not auto-tagged from the active capture game | Game arming is not yet proven on the user's desktop; clips still need manual/library filename game labels | M4/M7 |
| G3 | Compressed export needs reference-machine measurements; broader output controls remain open | Original-quality trim and software H.264 size presets (10 MB, 25 MB, custom 1–500 MB) are built. Generated MP4/MKV fixtures verify decoding, audio sync, cancellation, actual-size reporting, and collision safety. Compression is at most 720p/30 fps, copies AAC audio, and estimates rather than guarantees size; 30/60-second throughput measurements on representative PCs remain open | Local small-file export is available; broader codec/quality controls and speed claims remain limited | M4.2/M4.3 |
| G4 | macOS capture/audio is not implemented; Wayland capture is only partly verified | No ScreenCaptureKit/CoreAudio provider; Wayland portal and PipeWire pieces pass component tests but no real desktop chooser has delivered a frame | Limits supported-platform claims | M6 |
| G5 | Real-device audio, soak, and performance evidence are incomplete | Windows/AMD capture now has generated-window and generated-audio runs. Real audio devices, real-game impact, two-hour hardware soak and device-loss integration remain open | Limits reliability and release claims | M3/M5 |

The core workflow exists and can be exercised locally on Linux/X11 with synthetic capture and audio.
Do not mark platform support from compilation alone; attach evidence for the user workflow and
required failure cases. Do not publish completion percentages until a repeatable scorecard exists.

Vice reports sub-1% CPU for its GPU Screen Recorder backend, but this is not a measured ClipCraft
result or a universal expectation. GPU Screen Recorder documents X11 and Wayland support across
NVIDIA, AMD, and Intel with driver, permission, and compositor caveats; ClipCraft uses it as a
behavior reference only because its GPL-3.0-only non-Rust/FFmpeg runtime is outside project rules.
M5.2 plans a Rust-native GPU-surface path and requires real per-device measurements before any
compositor, GPU-vendor, or CPU-use claim.

Reference behavior from Vice now drives the engine's longest-match rule, user-managed game allowlist,
process observation, and automatic replay arming. The process watcher polls every two seconds only
in Game-only mode; it stores executable basenames and exact Steam IDs, not command lines or paths.
ClipRecorder's timestamped audio-gap policy is also ported in `clipcraft-record`, but the current
untimestamped `AudioSource` mixer does not use it. Upstream revisions, permission provenance, and
exact source files are recorded in [`upstream-ports.md`](upstream-ports.md).

## Performance review (2026-10-21)

M2.P1 removes encoded-video payload copies from clip snapshots, caches replay GOP boundaries and
avoids redundant full-frame RGBA clears and unscaled interpolation tables. On a Ryzen 7 5700X3D
Windows CPU microbenchmark, a ~150 MB video snapshot fell from 68.070 ms to 0.034 ms and 1080p
RGBA conversion with four threads fell from 1.392 ms to 0.638 ms. These are synthetic CPU timings,
not game-FPS evidence. Windows SDR native surfaces now avoid readback for compatible MF encoders;
[generated-window measurements](windows-performance.md) show an unresolved 1080p60 throughput limit.
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
- Capture: cursor, force window capture, HDR compatibility. Aspect ratio (game, stretch, black bars, crop to 16:9) moved to Advanced.
- General: launch at sign-in (Linux autostart entry, macOS launch agent, Windows Run key) and start minimized. Only the Linux path can be exercised in this container.

### Library organisation

- **Done:** favorites (heart on a card or in the clip view, right-click menu), collections (create, rename, delete from the sidebar; add or remove from a card's right-click menu or the clip view), and games (guessed from a sub-folder of the library or a recorder-style file name such as `Valorant 2024.03.01 - 21.04.mp4`; set by hand in the clip view). The sidebar lists All clips, Favorites, collections, and games with counts. All of it is saved in the local index, kept across rescans, and exposed as the `library.edit` command.
- **Limits:** the library scan is not recursive, so the sub-folder rule for games only helps once recursive scans exist; today games come from file names or your own choice. A plain file name with no date or ` - ` is not treated as a game. Collections hold clips by path, so a file moved outside ClipCraft leaves its collections.

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
  native SDR WGC-to-Media-Foundation route avoids that readback on the tested AMD combination,
  but both native and CPU runs still miss 1080p60. See [the report](windows-performance.md).
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
  720p30 + sound at +21 to +26 MB memory after warm-up over 3 to 4 minutes; the two-hour run has
  not been done. Tab moves a visible focus ring through every control and Enter activates; the
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
