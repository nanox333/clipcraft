# Features and platform limits

The full, detailed feature list and known limits. The [README](../README.md) has the short version.

## What works now

- **Settings:** a categorized, searchable Settings window with sliders, per-row reset and an Overview
  page of suggested profiles. New profiles have a four-step setup. Each shortcut action supports
  up to five bindings with +/remove controls; overlapping push-to-talk keys stay active until all
  are released.
- **Windows installation:** Rust packaging builds an offline per-user setup executable alongside
  the portable zip, with Start menu links, an optional desktop shortcut and an uninstaller that
  keeps clips and preferences. Downloads remain unsigned.
- **Clip deletion and saved pop-up:** right-click a clip (or use the delete button) to send it to the
  Recycle Bin on Windows; a small "clip saved" pop-up appears without taking focus from your game
  (Settings → Replay → Show a notification turns it off).

- **System tray (Windows):** a custom-styled tray menu (status, start/stop recording, save clip, screenshot, clips folder, library, settings, quit) and, by default, closing the window
  minimizes it so capture keeps running (Settings → General → Keep running in the tray). The taskbar
  button stays; a fully hidden window would stop capture.
- **Capture and replay:** choose a screen or window, record into a bounded replay buffer, and save
  clips with the global hotkey. Recording supports MP4/MKV, configurable quality and framing,
  replay length, post-roll, and storage limits. Linux/X11 has end-to-end synthetic-screen evidence;
  Windows/AMD has generated-window capture-to-MP4 evidence, including a native D3D11 path. Game-only
  mode is on by default: replay stays stopped until a recognized game process and capturable window
  are found, then stops when that process exits. Games settings also offer a timed desktop replay
  after a game closes (15 seconds–1 hour), or continuous desktop replay that automatically switches
  to game windows and back. Reopening a game cancels the timer; its next exit starts a fresh timer.
  Automatic source changes in the desktop modes never save a clip by themselves (a game closing or crashing does not make one); buffers remain bounded.
  Manual capture is also available.
- **Audio:** synchronized system audio and microphone recording, mixed or on separate AAC tracks,
  with levels, mute, push-to-talk, device-loss handling and optional offline microphone noise
  suppression (Settings › Audio). Suppression defaults off and preserves stereo; **Microphone is
  mono** explicitly mixes both sides. Restart clipping after changing capture processing. **Mic Test**
  plays your processed voice locally and meters it after suppression/volume/mono, with live setting
  changes and no saved audio (use headphones). The recorder path is tested with
  generated sources. A Windows default-microphone adapter/suppression check passes; real speech
  listening quality, speaker capture and physical-device disconnect tests remain open.
- **Playback:** open local clips in the app with sound, seek and step frames, adjust volume and
  playback speed, loop, select an audio track, and use fullscreen. Clip thumbnails are decoded and
  cached locally. Playback retains only its newest pending picture when the UI stalls; new seeks
  discard older pending/in-flight pictures rather than uploading a backlog (M11.1f). Queued playback
  and sound decoding now get priority over background thumbnails and waveforms (M11.1g). Rapid
  percentage-seek shortcuts keep the last key you pressed, including within one UI frame.
- **Library:** keep a rebuildable local index, restore the last folder, scan and search clips,
  sort them, view media details, reveal or relink files, and organize clips with favorites,
  collections, and game labels. Scans include subfolders (eight levels), skip symlinks, and run on
  a cancellable desktop worker; the CLI uses the same bounded traversal. Game-managed saves remember
  the game active when Save was pressed. Manual labels override that captured game, followed by
  folder/filename guesses; labels survive rescans and restarts in the local index.
  Search/sort results and sidebar counts are cached; the grid builds visible rows and supports
  keyboard navigation across offscreen clips. Static keyboard focus no longer keeps redrawing.
  Index writes run on one bounded background worker, with ordered edits and retry on failure.
  Closing waits for accepted writes; the app warns before discarding unsaved metadata. Abrupt
  termination can lose unacknowledged metadata; source clips remain untouched.
- **Quick trim and export:** mark a range in the player and save an original-quality trim, or
  choose an estimated 10 MB, 25 MB, or custom 1–500 MB small-file export. Compression uses the Rust
  software H.264 encoder, preserves AAC audio tracks, and reduces video to at most 720p/30 fps
  without upscaling. Both paths preserve the source and support progress/cancel; compressed exports
  report their actual size. Both writers use exclusive temporary files and no-replace publication.
  Their output filesystem must support hard links. Automatic copy names skip occupied partial/recovery
  files. When recording is set to the GPU encoder, small-file export tries it
  first, and the GPU decoder too, falling back to software on any trouble (untested on a real GPU
  so far). No upload or Discord account is involved.
- **GIF and loudness:** the same Trim panel can save a range of up to 30 seconds as a looping GIF
  (480 px wide, 12 fps, no sound) or even out its loudness to -16 LUFS (the picture is copied, the
  sound is re-encoded, and the result is never pushed into clipping). See
  [ADR 0018](adr/0018-gif-and-loudness-export.md); platform evidence is limited to Linux
  generated clips. GIF frame delays use cumulative centisecond timestamps so
  rates such as the default 12 fps preserve the requested duration.
  Joining clips is done in the small editor below.
- **Small sequence editor:** open its own **Editor** rail icon, **Import clips**, or **Send to editor**
  from Clips. Its resizable Library/Effects/Text panel, large preview and bottom timeline share the
  app's charcoal/lime styling, SVG icons (`assets/icons`) and embedded Inter typefaces. Append, split, trim, reorder, duplicate
  or remove cuts, with 64-step undo/redo, linked audio volume/mute, black/white fades and seven colour
  effects. Independent text layers can be moved, trimmed, split, duplicated, faded and recoloured.
  Preview and export share FilmCraft text rendering and the same audio fade envelope.
  They now also share a bounded compositor for aspect-fit scaling, colour effects, fades and text
  (M11.1i): compatible hardware uses wgpu, with automatic CPU fallback. Paused edits refresh on
  the decode worker; portrait/ultrawide sources keep their aspect ratio. Decode/font work and
  pixel upload/readback remain; physical-GPU speed and Metal/DX12 runtime are not verified here.
  The UI keeps only selected-text handle geometry; it no longer duplicates text textures or effects.
  Editor import resolves source bounds with the playback reader, so AAC padding cannot make a
  freshly captured clip's full-length timeline or export fail. Send to editor resolves these facts
  in the background before the clip can be added.
  Timeline strips sample the source pictures and linked audio waveforms; bounded background
  analysis keeps media work off the UI thread. Preview prepares the next source across cuts.
  **Export video** offers standard H.264 MP4 or estimated 10 MB, 25 MB and custom 1–500 MB presets,
  with actual size and encoder reporting, progress/cancel and GPU-encoder fallback to software.
  Choose **720p**, **1080p**, or **Source (up to 4K)** at **30 or 60 FPS**, with stereo AAC.
  Size presets can reduce resolution or bitrate; the dialog explains reductions and shows actual
  output settings. Source mode preserves the first clip's portrait or ultrawide framing.
  Exports run in the background with keyboard play/pause and frame stepping still available; progress
  and cancellation stay in the top toolbar, including in shorter windows. Cancellation shows its
  pending state until the worker stops. Matching-size exports skip redundant frame copies.
  Repeatable software-export measurements and their limits are in [the performance review](performance.md).
  Edits recover from internal local metadata after restart, including text, export preset and quality;
  undo history starts fresh. A restored draft never takes over launch: ClipCraft opens on Clips and
  the Editor tab reopens it. In short windows the editor page scrolls so the preview keeps its
  height. Originals remain untouched, and quick trim stays available. See
  [the editor guide](editor.md) for controls and limits.
- **Local match data:** optional CS2/Dota 2 GSI adapters and a League of Legends local API reader
  show selected map/match/player details in Settings → Games. All default off; Valve config setup
  preserves existing files. Snapshots are transient and are not stored with clips. Discord
  disclosure is controlled separately below.
  League transport requires Windows/macOS. Protocol/socket tests and Windows compilation are
  verified; live matches/native League TLS still need target-OS checks. See [setup and limits](game-integrations.md).
- **Discord Rich Presence:** opt-in activity, with separate game-name and selected match-context
  switches in General. A bounded local IPC worker handles update/clear/reconnect independently
  of capture. No media, paths, clip titles or account identifiers are published. Automatic setup uses a ClipCraft
  build with the shared ClipCraft identity configured once by the maintainer; end users then only
  enable the toggle. A Medal-inspired local card preview works without an ID, with original badges
  and optional readable match details. With game-name consent, CS2, Dota 2 and League use
  Discord-hosted game icons with the small CUT badge; publisher image files are not bundled.
  ClipCraft makes no artwork web requests, and its offline preview uses CUT. The shared public application ID is now included in ordinary
  builds. The CUT application icon is uploaded; real Windows profile checks show game artwork
  and the small CUT badge after Discord activity sharing is enabled. Other-account audience
  checks remain open. [Setup, exact fields and limits](discord-presence.md).
- **Settings and diagnostics:** recording, quality, audio, hotkey, storage, game allowlist, and
  general settings are stored locally. [348 known games](game-catalog.md) are recognized
  automatically, with per-game exclusions and a global switch in Settings › Games. The Settings
  Overview summarizes the setup and suggests fixes; the advanced toggle in General reveals the custom-game forms. The [custom-game form](settings.md) is a card like every other settings section, with padded,
  responsive fields and is advanced-only. Custom rules take priority over the built-in catalog and can match executable-name substrings and exact
  Steam IDs. A bounded background worker reads processes and [local Steam manifests](game-catalog.md)
  to recognize additional installed games without a download; custom rules and exclusions take priority.
  The app and CLI expose local diagnostics; the CLI can also list capture
  targets, record for a set duration, inspect the library, and trim clips.

## Known platform and format limits

Replay snapshots share immutable encoded-video bytes with the save job, avoiding a full-buffer
copy while recording. Replay insertion caches keyframe boundaries, and full-frame RGBA conversion
reuses output storage without redundant clears. Repeatable synthetic CPU measurements and remaining
GPU/capture limitations are documented in the [performance review](performance.md). Run
`cargo run --release -p clipcraft-record --example performance` to reproduce the CPU benchmarks;
they do not establish game-FPS impact or sustained capture performance.

- Linux/X11 capture and replay have been tested end to end in a virtual display (re-checked on 2026-10-26: start, F8 save, decode and library). X11 capture uses shared memory (about 5x faster at 1080p) and clips are not enlarged beyond the screen, so software encoding keeps up far better; hardware encoding is still the way to 1080p60 on a 4-core machine. Wayland support (see [docs/linux.md](linux.md) for what each Linux setup needs; global hotkeys on Wayland read the keyboards and the chosen screen is remembered) is
  partly verified; no real desktop portal chooser has delivered a frame yet.
- Windows/AMD native SDR capture avoids full-frame CPU readback at matching source/output size.
  Nonblocking latest-frame reads and a bounded SDR update-interval request have one-minute
  generated-window 1080p60 evidence, with fresh captures counted separately from repeated images.
  Standard-colour (SDR) capture is the default; the optional HDR compatibility setting retains CPU
  readback. A source of another size than the clip (scaling, cropping or letterboxing) is now
  scaled by the graphics card with black bars, when the encoder supports GPU input; incompatible
  encoders retain the CPU fallback. The scaling is covered by a Windows GPU test and a fake-encoder
  recorder test, but has not yet been measured on real hardware. An earlier 32 fps encoder
  wait stall did not reproduce before these changes, so its cause remains unexplained.
  See [Windows measurements and reproduction](windows-performance.md). A two-hour 1080p60
  soak, Windows hotkeys, real-game impact and other GPUs remain unverified; macOS capture is not
  implemented.
- Game-only capture is the default and stays idle until a recognized game is running. X11 and
  Windows window-to-process adapters are implemented; this environment verifies X11 with a
  synthetic window, not a real game. Wayland does not expose the
  process-to-window mapping used by Game source, and macOS capture is not available.
- The software H.264 encoder lives in this repository and is tuned for screen recording (`crates/filmcraft-h264enc`, see its `UPSTREAM.md`). If the graphics-card encoder fails or is busy, recording continues on the processor and the app asks the card again every 20 s or longer; the replay buffer restarts when it switches back.
- Linux hardware encoders have only been exercised against stand-ins here (the picture conversion they share is 2.5x faster and bit-identical; GPU scaling and zero-copy input for Linux are not done). Windows/AMD has the
  native capture-to-encode evidence described above, with CPU fallback for incompatible settings.
  Linux frames still pass through CPU-owned memory and conversion. Performance on gaming PCs and
  other GPUs needs measurement; Vice's reported sub-1% CPU use is not a ClipCraft performance claim.
- Playback supports more formats than trim/export. Original-quality trim copies samples to MP4;
  small-file export currently accepts H.264/AAC MP4/MOV/MKV, up to 30 minutes per job, and writes
  H.264 MP4. Target sizes are estimates, not guarantees about a changing Discord upload limit.
  Export needs an output filesystem with hard-link support for safe publication; software export
  speed on gaming PCs is unmeasured. WebM recording remains disabled; H.265 recording needs the graphics card (Windows).
- Game labels come from manual overrides, captured-game metadata or folder/filename guesses.
  Recognition does not retroactively label existing videos. There is no upload or link workflow.
- Real microphone/speaker capture, Windows device behavior, a real-game performance test, and the
  remaining release checks stay open. The ranked list is in [`docs/gaps.md`](gaps.md).

