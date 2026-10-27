# Changelog

## Unreleased

- **Installer and downloads:** Windows is a single `ClipCraft-Setup` download with an uninstaller;
  the command line tool is a separate download.
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
