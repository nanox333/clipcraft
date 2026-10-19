# ADR 0006: M2 capture, replay buffer, and the providers behind them

- **Status:** Accepted. Linux evidence is measured; Windows evidence is **not**: the Windows code
  compiles for `x86_64-pc-windows-gnu` but has never run on a Windows computer.
- **Date:** 2026-10-19

## Context

M2 is "display/window capture and a bounded replay buffer". [ADR 0002](0002-capture-platform-matrix.md)
planned Windows Graphics Capture and a software H.264 fallback, with every number unmeasured. This
container has no Windows and no GPU, but it has an X11 virtual display, so the pipeline can be
proved end to end on Linux and the Windows provider written and type-checked.

## Decision

Four small crates, each with one job and no `unsafe` of its own:

| Crate | Layer | Job |
| --- | --- | --- |
| `clipcraft-capture` | L1 | Targets, `CaptureFrame` (BGRA), the `CaptureSource` trait, a deterministic `SyntheticSource`, and the framing geometry (stretch, black bars, crop). |
| `clipcraft-record` | L3 | BGRA to 4:2:0 conversion with bilinear scaling, FilmCraft's pure-Rust H.264 encoder, the bounded `ReplayBuffer`, MP4 and MKV writers, and the `Recorder` threads. |
| `clipcraft-platform` | L5 | The operating-system providers and global hotkeys: X11 (`x11rb`, pure Rust), Windows Graphics Capture (`windows-capture`), and Windows hotkeys (`global-hotkey`). |
| `clipcraft-engine` | L4 | `CaptureSession`: starts a recording from the settings, saves a clip into the capture folder, and trims the folder to the storage limit. |

How it behaves:

- A capture thread polls the source at the frame rate and puts the newest picture in a one-slot
  mailbox. An encode thread takes it, converts, encodes, and appends to the replay buffer. A slow
  encoder therefore **drops** pictures (counted in the status), never queues them.
- Picture times are the recorder's own monotonic clock, in frames. Dropped frames become longer
  samples, so a clip keeps real time (it is variable frame rate when the encoder cannot keep up).
- The buffer keeps whole groups of pictures: at least the replay window, never more than its byte
  budget. A saved clip starts on the keyframe at or before the requested start, so it can be up
  to one keyframe interval longer than asked.
- A change in the picture size (a resized window) restarts the stream and clears the buffer,
  because old video would not match the new parameters.
- A clip is written to `name.partial` and renamed when complete. An existing file is never
  overwritten. A full disk or an unwritable folder is an error and leaves nothing behind.
- With a storage limit and "delete the oldest" selected, only clips ClipCraft named itself
  (`ClipCraft …`) are ever deleted, never the one just saved. With "stop saving" the save is
  refused once the folder is full.
- Sound is not recorded. That is M3.

## Dependencies added

- `x11rb` (MIT or Apache-2.0), Linux only, without its `unsafe` features.
- `windows-capture` (MIT) and `global-hotkey` (MIT or Apache-2.0), Windows only. They contain the
  Windows `unsafe`; ClipCraft code does not. The `cargo deny` licence, ban, and source checks pass
  with them.
- `fs4` (MIT or Apache-2.0) for free disk space.

## Consequences

- Every claim about Windows capture, Windows hotkeys, and exclusive-full-screen games stays open
  in `docs/gaps.md` until it is run on real hardware.
- Software H.264 encoding is the limit on weak computers; see the measurements in `docs/gaps.md`.
  Hardware encoders (FilmCraft's platform crate) are the way past it and are a separate decision.
- Wayland-only sessions and macOS have no provider and report that plainly.
