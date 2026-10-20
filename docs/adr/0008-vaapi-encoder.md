# ADR 0008: Hardware H.264 recording on AMD and Intel through VA-API (Linux)

- **Status:** Accepted. The code is tested against the libva headers' layout and a stand-in libva,
  but has **never run against a real AMD (Mesa) or Intel (iHD/i965) driver**.
- **Date:** 2026-10-20

## Context

On Linux, FilmCraft's NVENC covers NVIDIA only. AMD cards (Mesa `radeonsi`) and Intel graphics
(`iHD`, `i965`) expose H.264 encoding through VA-API. As on Windows ([ADR 0007](0007-media-foundation-encoder.md)),
the recorder's `HardwareEncoder` seam fits; FilmCraft is not changed.

## Decision

`clipcraft-platform` gets a `vaapi` module (Linux only):

- **libva is loaded at run time** (`libloading`: `libva.so.2`, `libva-drm.so.2`), so ClipCraft
  builds and runs without it, and a computer without libva reports "libva is not installed" and
  records on the CPU. No build-time dependency, no system header needed.
- **Devices.** Each `/dev/dri/renderD*` node is tried in order; the first whose driver can encode
  wins. Entry points: `EncSlice`, else `EncSliceLP` (what newer Intel parts offer).
- **Stream.** Constrained Baseline, CAVLC, one slice per picture, one reference frame (IPPP, no
  B-frames, so output order equals input order), an IDR every `keyframe_frames` pictures. CBR or
  VBR from what the driver offers. The driver writes slice headers. When it supports packed
  headers, ClipCraft supplies the SPS and PPS (`h264_headers`, checked against FilmCraft's
  parser); otherwise the driver writes them. The `avcC` record uses the parameter sets found in the
  driver's own first picture, falling back to ClipCraft's.
- **Pixels.** RGBA becomes NV12 (`h264_stream::rgba_to_nv12`), is written through `vaDeriveImage`
  honouring the driver's pitches, and the last row and column are repeated into the padding up to
  the 16-pixel macroblock grid; the SPS crops the padding away.
- **`unsafe`** is confined to `vaapi/` (`#![allow(unsafe_code)]` there only, every block carries a
  `// SAFETY:` comment, enforced by `clippy::undocumented_unsafe_blocks`). All resources are owned
  by one value whose `Drop` releases them in libva's required order, whichever step failed; sizes
  and segment chains the driver reports are bounded before use.
- **Verification.** `vaapi/ffi.rs` tests every structure size and written field offset against
  numbers produced by compiling libva's `va.h` and `va_enc_h264.h`. A table of stand-in libva
  functions tests call order, buffer contents, upload, cadence, and that a failure injected at each
  libva call leaves no buffer, surface, context, config, image, mapping, or display behind.

## Consequences

- A real driver may disagree with the stand-in. Likely first failures: a driver that insists on
  packed slice headers, one that rejects `max_num_ref_frames = 1` with a given level, or one that
  refuses the low-power entry point with CBR. Each surfaces as an error naming the failed libva
  call; the recorder then uses the software encoder and the status says why.
- The first thing to check on a real computer: the Quality tab names `AMD VA-API` or `Intel
  VA-API`, and a recorded clip plays back in other players.
- HDR and 10-bit input are not handled; recording hands this encoder 8-bit pictures.
