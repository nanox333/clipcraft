# Changelog

## Unreleased

- **Removed:** the automatic clip when a game closes or crashes. ClipCraft no longer saves a clip
  by itself when an app closes; clips are saved only when you press the save shortcut.

- **Yellow border:** ClipCraft now asks Windows for borderless capture access, so the yellow frame Windows
  draws around captured windows no longer appears around what ClipCraft records.
- **Fixed:** the "found <game> running, but this desktop did not expose its game window" message now
  goes away when the game closes.

## 0.2.0 — 2026-10-28

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
