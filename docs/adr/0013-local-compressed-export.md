# ADR 0013: Offline exports with estimated file-size targets

**Status:** accepted · **Work:** M4.3 · **Date:** 2026-10-21

## Decision

**Addendum (M4.4):** when the recording setting selects the GPU encoder, `clip.export` first tries the
hardware encoder behind the recorder's `HardwareProvider` seam (through the export crate's own
small `FastEncoder` trait, so the export layer does not depend on the recorder). Pictures go in as
RGBA, packets must come out one per picture in order, and any failure restarts the export with the
software encoder described below. The report names the encoder used. FilmCraft's OS hardware
decoders are registered at start-up but switched off globally; an export that may use the GPU
switches them on only while it opens its decoder.

Keep the sample-copy trim from ADR 0010 as the default. Add a separate `clip.export` engine
command, used by the trim panel and local CLI, for estimated 10 MB, 25 MB, and custom 1–500 MB
exports. MB means 1,000,000 bytes. This creates a normal local MP4; no upload, account, or network
operation is part of export.

Reuse the already reviewed, pinned FilmCraft Rust decoder, H.264 encoder, RGBA-to-YUV conversion,
and MP4 muxer (ADR 0004), plus the workspace's existing image resize implementation. No new
upstream revision, license exception, native dependency, or platform unsafe code is introduced.

The initial implementation accepts H.264 video and up to 16 AAC audio tracks from supported
MP4/MOV/MKV sources. It copies the selected AAC samples and edit offsets, reserving their exact
bytes before allocating a video bitrate. It reserves another 10% of the target plus 64 KB for
overhead and rate variation. Unworkable audio/video budgets fail with an actionable message.

Video is decoded and re-encoded sequentially with a two-thread software H.264 Baseline encoder.
The maximum output is 1280×720 at 30 fps; lower source rates are kept rational, size is never
upscaled, and lower budgets reduce the resolution further. Each job is limited to 30 minutes,
54,000 frames, and source dimensions up to 7680 pixels per edge and 33,177,600 pixels. A target
larger than the source does not raise the estimated source bitrate. No full-clip decoded frame
queue is retained.

Create the temporary output exclusively and own its cleanup only after creation succeeds. Flush
and sync it before atomically linking it to a destination that must not exist. Remove the
temporary link afterward. This also prevents a destination created during encoding from being
overwritten. Filesystems without hard-link support cannot use this export path and should use
another output location. Decoder/encoder panics are contained as job failures with cleanup.

## Limits and evidence

Size is an estimate; a one-pass variable-bitrate encoder can exceed it. Report actual bytes and
whether they fit the target, rather than promising compliance with a third-party upload limit.
Copied audio can dominate the budget; this slice does not reduce audio bitrate or mix tracks.
Explicit HDR and non-square-pixel sources are refused. HDR mastering, arbitrary output codecs,
hardware export, and custom resolution/FPS controls are outside this initial slice. Container
metadata is not copied wholesale; supported display rotations are applied by the existing decoder.

Rust-generated MP4/MKV fixtures verify output decoding, trim timing, two audio tracks, click sync,
30 fps output, source preservation, progress/cancel, malformed input, and collision cleanup.
FFmpeg is an optional external oracle only. Native desktop screenshots and keyboard checks cover
the export selector. Software throughput measurements do not establish capture/GPU performance.
