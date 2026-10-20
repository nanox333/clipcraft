# Known gaps

**Status:** active build baseline · **Last reviewed:** 2026-10-19 · **Target:** local Windows capture-to-export alpha

This is the ranked work list, not a parity claim. The workspace, UI shell, time primitives, scanner,
and first command tests exist; capture-to-export is still unimplemented. Estimates and performance
claims remain TBD until the platform spikes provide evidence. Rank by user impact, frequency, and
evidence. Update this file whenever a gap's status, workaround, or estimate changes.

| Rank | Gap | Evidence / current limitation | Impact | Work |
| ---: | --- | --- | --- | --- |
| G0 | M0 hardware evidence and Windows CI remain open | Workspace, shell, CLI, time crate (including audio sample math and a one-hour synthetic A/V alignment test), scanner, and layer/asset/license checks exist; the gate passes locally on Linux. The capability matrix ([ADR 0002](adr/0002-capture-platform-matrix.md)) is documentation only, and the Windows CI job has not run | Limits platform and release claims | M0 close-out |
| G1 | Windows capture API and hardware matrix are undecided | Must measure monitor/window capture, audio APIs, permissions, and encoder availability | Core app cannot record reliably | M0 spike, then M2/M3 |
| G2 | Replay buffer and A/V synchronization are unimplemented | Exact rational time primitives exist; no segment ring, bounded queues, or A/V synchronization tests | Instant clipping is the core workflow | M1/M2/M3 |
| G3 | Persistent library and storage controls are incomplete | The desktop can browse one local folder, scan it, search titles, sort clips, copy paths, and reveal files; a versioned local index restores the last folder; MP4/MOV duration and stream facts are probed; clips play with sound; thumbnails are cached. Missing: WebM/MKV probing, a storage quota, full-disk behavior, and any UI for the stored favorites and tags | Users still have no durable library organization or storage protection | M1.2/M4.1/M5 |
| G4 | Editor and export quality are unverified | No cut/trim model, MP4 exporter, or pinned media corpus | Cannot deliver dependable clips | M1/M4 |
| G5 | Mac and Linux capture are not implemented | No ScreenCaptureKit/CoreAudio or PipeWire/portal provider | Initial release is Windows-only | M6 |

G0 tracks remaining workspace and gate work. The capture capability spike in G1 is the next blocker
for recording features. Do not mark a gap done because code compiles; attach evidence for its user
workflow and required failure cases. Do not publish percentages until a repeatable scorecard exists.

## Recording output options

- **Done:** recording preferences (container MP4/MKV, game or desktop source, 16:9 / 4:3 / source shape with stretch, black bars, or crop, resolution, frame rate, replay length) are modelled, validated, saved locally (`recording.load_settings` / `recording.save_settings`, `clipcraft-cli recording settings`), and shown in Settings. The reframe geometry is unit-tested.
- **WebM unavailable:** FilmCraft has VP9 and Opus decoders only. Pure-Rust encoders exist on crates.io (`rusty_vp9`, `opus-pure`, `rav1e`) but are young, carry many `unsafe` blocks, and have not been vetted. WebM stays greyed out until one passes a spike (decode check with ffprobe/ffmpeg) and a licence and `unsafe` review.
- **Not done:** capture itself (full-screen game and desktop sources), and exporting a clip with the chosen format and framing. Neither can be verified in a headless container.
- **H.265:** FilmCraft has a pure-Rust HEVC *decoder* only. Encoding is hardware-only (VideoToolbox on macOS, NVENC on Windows, via `filmcraft-platform`, which contains `unsafe` FFI), so H.265 is greyed out in Settings until that path is adopted and checked on real hardware.

### Recording settings (applied by M2 where noted below)

- Quality tiles (Low 360p24, Standard 720p60, High 1080p60, Custom with resolution, FPS, bitrate, encoder, codec).
- Hotkeys for save clip, long recording, mute microphone, push to talk, screenshot, with conflict detection. They are saved but not yet registered with the operating system; global key capture needs the capture work.
- Audio: PC audio and microphone toggles, output and microphone device pickers, a live microphone level test, push to talk, mono microphone, separate tracks, volumes.
- Storage: capture folder, usage and free space, a storage limit, and a delete-oldest or stop policy. The cleanup plan is tested; nothing is deleted until recording exists.
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

- **Windows.** The Graphics Capture provider and the Windows hotkeys compile
  (`cargo check --target x86_64-pc-windows-gnu`) but have never run. Exclusive-full-screen games,
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
  recorded, and macOS has none; resampling is linear; per-application audio is not separated; the screenshot hotkey is
  saved but not wired; push-to-talk works through the global hotkey only (not when ClipCraft
  has the focus). Sound and picture of a clip can differ by up to a frame at the very end.
- **Settings not yet applied:** H.265 and WebM.
- **Speed.** Measured on a shared 4-core container with a software-rendered video playing on the
  same screen, using the Fast preset: 360p30 keeps up (10 ms per picture); 720p30 keeps up (25 ms;
  241 of 241 pictures); 720p60 encodes about two of every three pictures (it drops a third); 1080p
  at any rate is not sustainable in software. Pixel conversion alone is 4 to 11 ms at 1080p. A
  real gaming computer is faster and the game also competes for it; hardware encoders are the way
  to 1080p60 and are not wired. The top bar shows when more than 5% of pictures are dropped.
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
