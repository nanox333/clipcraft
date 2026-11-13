# How ClipCraft works

A walkthrough of what happens between pressing the clip hotkey and finding a file, and between
dropping a file in the library and exporting an edit. It follows the code as it is. For the
dependency rules see [architecture.md](architecture.md); for what is verified on which system see
[gaps.md](gaps.md); for every feature and limit see [features.md](features.md).

## The shape of the program

ClipCraft is one desktop program (`clipcraft`) and one command-line program (`clipcraft-cli`).
Both are thin: they draw a screen or parse arguments and then call **engine commands**
(`crates/clipcraft-engine`). A command has a stable ID such as `library.rename`, `clip.export` or
`capture.start`, one implementation, and one result type, so the window and the CLI can never
disagree about what an operation does. There is no server and no local control port.

```text
 window (egui)  ─┐
                 ├─►  engine commands  ─►  recorder / export / player / index files
 CLI ────────────┘          │
                            └─►  platform adapters (screen capture, hotkeys, GPU encoders, tray)
```

Layers only depend downward (time → capture model → recorder/export/player → engine → platform
and UI → apps). `cargo xtask layers` enforces it. Everything stays on the computer: no account,
upload, telemetry or share link. The only network use is the optional update check
(Settings → General), which can be turned off.

Where things are stored:

| What | Where |
| --- | --- |
| Clips | the capture folder (default `Videos/ClipCraft`, `Movies/ClipCraft` on macOS) |
| Settings | `recording-settings.json` in the per-user data folder |
| Library index (favorites, collections, game labels, probes) | `library-index.json`, same folder |
| Thumbnails | `thumbnails/`, same folder |

The data folder is `%APPDATA%` on Windows, `~/Library/Application Support` on macOS and
`$XDG_DATA_HOME` (or `~/.local/share`) on Linux.

## Recording: from screen to replay buffer

1. **Pick a source.** `clipcraft-platform` lists screens and windows and opens one as a
   `CaptureSource` (the trait lives in `clipcraft-capture`, which has no operating-system code):
   Windows Graphics Capture on Windows, the X11 core protocol on Linux, and the desktop's
   ScreenCast portal with PipeWire on Wayland, where the desktop shows its own chooser. macOS has no capture
   provider yet.
2. **Start the recorder** (`clipcraft-record`). `CaptureSession::start_with_audio` (engine) turns
   the settings into a `RecordConfig` and starts three kinds of thread:
   - a *capture loop* that asks the source for a picture on a fixed schedule, always keeping only
     the newest picture (an overloaded encoder drops pictures instead of falling behind);
   - an *encode loop* that fits each picture into the clip's frame (same shape, black bars, crop or
     stretch, never taller than the source) and encodes H.264 — hardware where the platform offers
     it (Media Foundation / D3D11 on Windows, VAAPI on Linux), otherwise the built-in pure-Rust
     encoder in `filmcraft-h264enc`. If a hardware encoder fails the recorder carries on in
     software and retries the card later;
   - an *audio loop* that mixes or separates system sound and the microphone into AAC, with
     optional offline noise suppression.
3. **Buffer.** Encoded pictures go into a `ReplayBuffer`: the newest N seconds kept as whole groups
   of pictures that start on a keyframe, bounded in time *and* bytes, so memory never grows. The
   oldest whole group is dropped as new ones arrive. Sound has its own bounded buffer.
4. **Pause.** The recorder's clock can stop (`set_paused`): nothing is captured or kept, sound is
   read and discarded, and later pictures carry on from the same time so a clip has no frozen gap.
5. **Resets.** A change of picture size (when the stream cannot be kept), a silence longer than the
   gap setting, or a hardware/software encoder switch starts a new stream and clears the buffer.
   The capture bar reports it. When the source changes in game-only mode the stream is *kept*
   (see below).

The picture timeline uses real time, not a frame counter, so sound and video stay in step.

## Saving a clip

Pressing the hotkey calls `prepare_save` on the session and then runs a `SaveJob` on another
thread so the window and the game are never blocked.

1. **Checks before anything is written:** the storage limit (when "delete oldest" is off), and
   that the disk has room for what is buffered. An MP4 needs twice the space while it is written,
   because a crash-safe copy is made first.
2. **Post-roll:** the job waits the configured number of seconds so the moment *after* the key
   press is included. If recording stops during the wait it saves what it has at once.
3. **Write:** the newest `replay + post-roll` seconds (or a clip-length shortcut's own length) are
   taken from the buffer starting at the keyframe at or before that point, and written as MP4 or
   MKV with a unique name (`ClipCraft 2026-10-19 21-04-55`, or `<Game> …` when a game is known).
   Nothing is re-encoded; the buffered packets are muxed as they are. A failed or interrupted
   write leaves no half file, and unfinished files from a crash are cleaned at the next start.
4. **Trim the folder:** with "delete oldest" on, the oldest ClipCraft clips go until the folder is
   under its limit. Clips that ClipCraft did not make are never deleted.
5. **Tell the library:** the finished clip is registered in the library index with its game label.

## Hotkeys

The hotkey layer (`clipcraft-platform`) registers global shortcuts with the operating system
(Windows hot-keys, X11 key grabs, and the GlobalShortcuts portal on Wayland, with an opt-in keyboard-device fallback). Each action can have up
to five bindings; *clip-length shortcuts* map extra keys to their own length. When the desktop
refuses a global key, the window still reacts to the same keys while it has focus. Push-to-talk
holds are combined across alternatives and released if the listener is lost.

## Game-only recording

With "game-only capture" on (the default), recording is armed by what is running instead of by
hand. Most of this lives in `apps/clipcraft/src/capture/games.rs`, with the decision logic
(`GameArmingState`) in the engine so it can be tested without a screen.

1. **Discovery.** One worker thread (`game_scan.rs`) lists running programs every few seconds and
   matches them against the bundled game catalog, the person's own rules and (optionally) the
   games installed in Steam, whose local manifest files are re-read only when something changes.
   Nothing leaves the computer.
2. **Finding the window.** A recognized process does not always own its window (launchers start the
   game as a child, anti-cheat bootstraps replace windows). The window is looked for in the
   process, the processes it started and the other processes of the same game, and then by window
   class name. If none is found and "record the screen when the game window is missing" is on, the
   screen is recorded until a window appears.
3. **Arming.** `GameArmingState` turns what it sees into one decision: start for a game, start the
   desktop (if a desktop mode is chosen), or stop. Starts are retried at most every ten seconds so
   a failing window cannot spin.
4. **Switching without losing the buffer.** When capture has to move — a game window closes and
   the desktop takes over, a game opens, a launcher window is replaced by the game's own — the
   *same recording* is kept. The recorder is told to wait when its source is lost
   (`keep_through_source_loss`) and gets a new source (`replace_source`) instead of being stopped.
   The buffer, the encoder and the sound carry on; a picture of another size is fitted inside the
   existing clip frame with black bars so one stream never mixes sizes. If that is not possible
   (the Wayland chooser, a source that will not open, or more than the gap setting of silence) the
   old behaviour applies: stop, clear and start again.
5. A game closing never saves a clip by itself; clips are only saved when asked.

## The library

The library is a view of a folder plus a small index.

- **Scan:** a worker walks the folder (bounded depth and count) for video files. The result is
  merged with the saved index, so favorites, collections, game labels and probe results survive;
  files that disappeared stay listed as *offline* until removed or found again.
- **Probe and thumbnails:** other workers read each clip's container headers (never the picture
  data) for duration, size, frame rate and codecs, and make thumbnails into the cache. Both stop
  when the folder changes, and bad files are reported, not fatal.
- **Edits to metadata** (`MetaEdit`): favorite, game, collections, relinking a moved file,
  forgetting an offline entry. The window applies the edit to its copy at once and the index file
  is written by a single writer thread, so the screen never waits for the disk.
- **Rename** (`library.rename`): same folder, same extension, never replaces a file; the entry is
  moved in the index so favorite, collections and game stay.
- **Delete:** always asks, then moves the file to the Recycle Bin / trash (retried for two seconds
  in case the player still holds it). ClipCraft itself never deletes for good except when trimming
  its own folder to the storage limit.

## Playing, trimming and exporting

- **Playback** (`clipcraft-player`) decodes on a worker thread against a wall clock and hands
  RGBA frames to the window. If decoding falls behind, frames are skipped and the clock keeps its
  pace; a damaged file or a decoder panic is reported as a failure and cannot freeze the window.
  Sound is decoded at the output device's rate into a bounded feed that the device callback drains
  (`clipcraft-audio-out`).
- **Trim** (`clipcraft-export`): copies the pictures from the keyframe at or before the start and
  uses an edit list so playback starts on the exact frame; sound is copied the same way. Nothing
  is decoded or re-encoded, so it is instant and lossless.
- **Size-targeted export** (`compress.rs`): re-encodes to an estimated size (10 MB, 25 MB, custom).
  If the first result is over the target, it is encoded once more at a lower bitrate (and a smaller
  picture for a big overshoot) *before* anything is saved; the result says when that happened.
- **GIF and loudness:** `clip.export_gif` writes a short looping GIF; `clip.normalize` brings the
  sound to a common loudness with one gain and leaves the picture as it is.
- **Never overwrite:** every export writes a new file beside the original, and cancelling leaves
  nothing behind.

## The editor

The editor (`apps/clipcraft/src/editor*.rs`, `crates/clipcraft-engine/src/editor.rs`) is a small
sequence editor: clips on one linked video/audio timeline, split, delete, reorder, duplicate,
fades, colour looks and text layers, with undo and redo. The *edit session* in the engine is plain
data plus exact time (`Tick`, rational frame rates — no floating-point seconds in edit math). The
window shows a preview through the same player as the library, prepares the next clip's decoder
ahead of a cut, and exports through `clipcraft-export` with a quality and size choice. The
session is saved to a recovery draft so a crash does not lose it. See [editor.md](editor.md).

## Settings

`RecordingSettings` (engine) is the single settings structure. It is validated and clamped
(`sanitized`) whenever it is loaded, saved or used, so a hand-edited file cannot start a recording
with absurd values. The Settings page edits a copy; changes apply when saved (or per row), and
"Cancel changes" restores the snapshot taken when the page opened. A first-run setup edits a draft
and saves it once. A damaged settings file is reported and kept, not overwritten. See
[settings.md](settings.md).

## Platform code and safety rules

- Rust only: no JavaScript, web view or non-Rust sidecar. Operating-system bindings, and the only
  `unsafe` code, live in `clipcraft-platform` (and the audio output crate), each with a
  `// SAFETY:` comment.
- Non-test code does not panic: hostile or damaged input (clip headers, settings, index files) is
  bounded and reported as an error. Queues are bounded by count and bytes.
- Windows installer and updater: the installer is an offline per-user program that writes Start
  menu links and an uninstaller; the update check downloads over HTTPS and verifies a checksum
  before offering to run it. The `.exe` files carry the ClipCraft icon through a resource built
  by `build_support/windows_icon.rs`.

## Where to look when something breaks

| Symptom | Start here |
| --- | --- |
| Nothing records | `capture.rs` (`start_capture`), `capture/games.rs` (game-only), `clipcraft-platform` |
| Clip is missing the last seconds | post-roll in `SaveJob::run`, `ReplayBuffer::snapshot` |
| Buffer cleared unexpectedly | `RecordStatus.buffer_resets`, `encode_one`/`new_active` in `recorder.rs` |
| A game is not recognized | `game_scan.rs`, `game_catalog_data.rs`, Settings → Games |
| Library out of date | `library_scan.rs`, `library_store.rs`, `index.rs` |
| Export too big / wrong | `clipcraft-export/src/compress.rs` |
| A setting resets | `RecordingSettings::sanitized` in `recording.rs` |

Every command, its ID and its behaviour: `clipcraft-cli commands`. Tests and how to run them:
[testing.md](testing.md).
