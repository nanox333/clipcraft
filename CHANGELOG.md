# Changelog

## Unreleased

- **Cleaner clip cards:** the Clips page shows the picture with the name and details under it; Send to editor, Show in folder and Favorite appear as round buttons on the picture when you point at a clip.
- **Consistent layout:** spacing, corner radii, text sizes and control heights now come from one
  shared scale, so buttons, search boxes, folder fields, pills and settings rows line up across the
  Clips, Editor, Settings and setup screens.
- **Smaller software-encoded desktop clips:** the built-in H.264 encoder now uses split-block motion
  prediction, making screen recordings about a third smaller at the same quality for roughly 1.5x
  the encoding time on desktop content (game footage is unchanged).

## 0.2.0 — 2026-10-29

- **Wayland shortcuts:** use the desktop Global Shortcuts portal when available, show the assigned
  keys, and offer a default-off keyboard-device fallback. Real Wayland desktops still need testing.
- **Replay status:** see the buffered duration and get a warning when switching sources or
  restarting the recording stream clears buffered video.
- **Bug reports:** prepare and review a report locally in Settings → About, then optionally open
  the GitHub form to submit it yourself.
- **Fixed:** playback steps over a picture the decoder cannot read (up to 30 in a row) instead of
  stopping the whole clip.
- **Fixed:** a cut no longer moves sound that starts late earlier than the picture; sound that ends
  before the cut is left out; GIFs stop at the end of the clip.
- **Fixed:** surround game sound is folded to stereo with the centre channel kept; sound read in
  split pieces no longer shifts channels.
- **Fixed:** recording survives brief capture errors (about five seconds) instead of stopping at the first.
- **Fixed:** library edits, scans and probes can no longer overwrite each other's changes; a second
  damaged settings or library file no longer replaces the first kept copy.
- **Fixed:** only files named like ClipCraft's own timestamped clips are removed by "delete oldest".
- **Fixed:** X11 window capture picks the largest visible window of a game, keeps shared-memory
  capture after a single hiccup, and reads very large screens; the update check only contacts
  GitHub's release hosts; `Ctrl++` and lower-case modifiers work in shortcuts.
- **Fixed:** a stray local request no longer hides working game data; Steam files with `\n` or `\t`
  escapes are read; the CLI says what is wrong with JSON input.
- **Fixed:** screens running in 30-bit colour are no longer captured with wrong colours on X11.
- **Fixed:** after you stop game capture yourself, automatic capture resumes once the game has
  closed (it stayed paused forever in "after game" desktop mode).
- **Fixed:** updating no longer fails after eight installs; the oldest version is removed to make room.
- **Changed:** pressing Enter in the delete-clip box no longer confirms the delete.
- **Fixed:** saving a clip or screenshot never replaces an existing file or shares another save's
  temporary file; export no longer needs a filesystem with hard links (exFAT and FAT drives work).
- **Fixed:** a clip whose sound ends before its picture can be cut after the sound has ended.
- **Fixed:** renaming a collection now updates its clips in every library folder, not only the open one.
- **Fixed:** start-up recovery no longer deletes a clip that still plays or a crash-safe copy it could
  not yet rebuild; "delete oldest" does nothing when the folder listing was cut short.
- **Fixed:** a game that closes its connection early no longer leaves the game-data listener in a
  permanent error state.

## 0.1.0 — first release

ClipCraft is an offline, Rust-only clip recorder and lightweight editor. Nothing it records leaves
your computer.

- **Record:** screen or window capture with a replay buffer, hardware (H.264 and H.265) and software
  encoding, system and microphone audio, and offline microphone noise suppression.
- **Library and editor:** local clip library, smooth 60 fps playback, trimming and export.
- **Desktop app:** redesigned Settings with search, system tray, delete to the Recycle Bin with a
  right-click menu, and a saved-clip pop-up that never steals focus from your game.
- **Command line:** `clipcraft-cli` runs the same commands as the app, plus `diagnose`.

Known limits: downloads are not code-signed (Windows SmartScreen shows a warning); HDR capture is not
yet supported. Check each download against `SHA256SUMS.txt`.
