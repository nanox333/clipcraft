# Changelog

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
