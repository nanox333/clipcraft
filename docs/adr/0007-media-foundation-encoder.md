# ADR 0007: Hardware H.264 recording on AMD and Intel through Media Foundation

- **Status:** Accepted. The code compiles for Windows and its stream handling is unit tested, but it
  has **never run on a Windows computer or any AMD, Intel, or NVIDIA encoder**.
- **Date:** 2026-10-19

## Context

Recording needs a hardware encoder on most computers: the software encoder keeps up at 720p30 and
falls behind above that ([ADR 0006](0006-m2-capture-and-replay-buffer.md)). FilmCraft's hardware
encoders cover NVIDIA (NVENC) and Apple (VideoToolbox) only. An AMD graphics card, and an Intel one,
had no way to record with hardware. Both vendors install a hardware H.264 encoder that Windows lists
as a Media Foundation transform (MFT). That is the one interface that reaches all three vendors.

## Decision

`clipcraft-platform` gets a Media Foundation H.264 encoder (`mf_encoder.rs`) behind the recorder's
safe `HardwareEncoder` trait. On Windows the recorder tries NVIDIA's encoder first (through
FilmCraft), then the first hardware MFT Windows lists that accepts the request. Any failure, at
start or later, hands the stream to the software encoder and says why in the status.

Pure logic lives in `h264_stream.rs` and is tested on every platform: Annex B to length-prefixed
conversion, the `avcC` decoder configuration record (checked byte for byte against the software
encoder's own), and RGBA to NV12.

## `unsafe`

This is the only `unsafe` in ClipCraft's own code, as `AGENTS.md` allows: confined to
`clipcraft-platform`, in one file, every block with a `// SAFETY:` comment, and the lint
`clippy::undocumented_unsafe_blocks` warns on any that lacks one. It cannot be reached except through
the trait, which returns `Result` everywhere. The unsafe parts are:

- Loading six functions from `mfplat.dll` at run time (`LoadLibraryW`, `GetProcAddress`) instead of
  linking them, so ClipCraft still starts on Windows editions without the Media Feature Pack.
- Calling Media Foundation and COM interfaces, and copying picture bytes into and out of the
  buffers they own.
- Two `unsafe impl`s (`Send` for the encoder, `Send` and `Sync` for the function table) with the
  reasons stated beside them.

## Behaviour

- Pictures go in as NV12 in system memory; the encoder runs in the Baseline profile with B-frames
  off, so pictures come out in order. If an encoder refuses the profile tag it is dropped and
  reordering is checked on every output picture instead. A reordered picture counts as failure.
- Asynchronous encoders (the usual kind for hardware) are driven by their need-input and have-output
  events with a two-second stall limit; synchronous ones by input then output.
- Rate control is constant or variable bitrate, a keyframe every keyframe-interval, low latency on.
  Each setting is best effort, because encoders may refuse some.
- Output is converted from Annex B and the parameter sets are taken from the stream (or the output
  type's sequence header) to build the `avcC` record the MP4 and Matroska writers need.

## Consequences

- The first thing to check on a real AMD or Intel computer: the Quality tab names the encoder
  (`GPU (Media Foundation: …)`), a recording runs without dropped frames, and the saved clip plays.
  Until someone does, `docs/gaps.md` keeps this open.
- Known unknowns: whether a given driver accepts system-memory NV12 input without a Direct3D
  device manager, whether it accepts the Baseline profile, and how its rate control behaves under
  the one-second constant-bitrate setting. Each failure falls back to the CPU rather than
  producing a bad clip.
- H.265 is not covered; the file writers are H.264 only.
