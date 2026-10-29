# Changelog

## Unreleased

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
- **Smaller software-encoded desktop clips:** the built-in H.264 encoder now uses split-block motion
  prediction, making screen recordings about a third smaller at the same quality for roughly 1.5x
  the encoding time on desktop content (game footage is unchanged).
- **Faster software encoding:** the built-in H.264 encoder is 1.3-3x faster on screen content, so computers
  without a hardware encoder drop fewer frames.
- **Removed:** the automatic clip when a game closes or crashes. ClipCraft no longer saves a clip
  by itself when an app closes; clips are saved only when you press the save shortcut.

- **Yellow border:** ClipCraft now asks Windows for borderless capture access, so the yellow frame Windows
  draws around captured windows no longer appears around what ClipCraft records.
- **Fixed:** the "found <game> running, but this desktop did not expose its game window" message now
  goes away when the game closes.

- **Installer and downloads:** Windows is a single `ClipCraft-Setup` download with an uninstaller;
  the command line tool is a separate download.
- **Look:** the first-run setup and the Windows installer/uninstaller were redesigned to match the
  app: a seven-step setup (welcome, quality, capture, sound, storage, shortcuts, summary) with big visual choices, keycap shortcuts, a smooth fade between pages and a check of your graphics card, and a branded installer with a progress page and a
  clear done screen.
- **Updates:** ClipCraft can tell you when a newer version is published and install it after you
  press Update now (checksum-verified). Turn the check off in Settings → General.
- **Clips:** new clips are named `<Game> <date> <time>` and filed in a folder per game (`Desktop`
  when no game is running). Deleting a clip removes it from the Clips tab, the count after "Clips" is
  gone, there are more sort orders, and the clips folder is chosen with one click.
- **Editor:** Import clips now shows your library first, with a button to browse other files; the
  edit name is a visible field and stays "Unnamed edit" when empty.
- **Settings:** Cancel changes bar, one place for custom games (no more Show advanced settings),
  clearer Replay and Capture pages, and a clear "Clipping" indicator while recording.
- **Tray:** minimized to the tray, ClipCraft has no taskbar button; hotkeys keep working while it is
  in the tray or behind a game.

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
