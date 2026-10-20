# ADR 0011: Never lose a saved clip

- **Status:** Accepted. Interruptions are simulated with cut-off files and a real full disk (tmpfs);
  the app has not been killed mid-write on a real machine.
- **Date:** 2026-10-20

## Context

People's clips are the product. The replay buffer is in memory by design and cannot survive a
crash, but everything already on disk must, and a failed save must be understandable.

## Decision

- **Write, then rename.** Clips, trimmed clips, settings and the library index are written to a
  temporary file and renamed into place only when complete; failures remove the temporary file.
  Nothing is ever written over an existing clip.
- **Clean up after a crash.** On launch, `.partial` files that ClipCraft made (clip names, and
  `(trimmed)` copies) and that are older than 30 s are handled in the capture and library folders:
  a Matroska one that still reads is kept as a clip; anything else cannot be played and is removed.
  Files ClipCraft did not make are never touched. The person is told.
- **Keep what cannot be read.** A damaged or too-new settings file or library index is moved to
  `<name>.damaged` before the defaults or a rescan replace it.
- **Disk full.** A save checks the free space against what is buffered and refuses first with a
  clear message, writing nothing. If the disk fills during a write anyway, the writers' I/O errors
  are mapped to "disk is full" (not a vague failure), the partial file is removed, earlier clips
  are untouched, recording continues, and older clips are never deleted to make room.

## Consequences

- A crash during clipping still loses the unsaved buffer. Saving continuously to disk was
  rejected: it is the "full recording" ClipCraft deliberately does not make.
- An MP4 cut off mid-write cannot be recovered (its index is written last); the window is the
  fraction of a second a save takes.
