# ADR 0017: GIF output, lossless clip merge, and loudness evening

**Status:** accepted · **Work:** M4.4 · **Date:** 2026-10-22

## Decision

Add three offline engine commands beside `clip.trim` and `clip.export`, used by the player's trim
panel, the library, and the CLI. Like every export they write a new normal local file, never touch
the source, never replace an existing file, clean up on cancel or failure, and need no network.

- **`clip.export_gif`** writes a looping animated GIF of a range of 0.1 to 30 seconds, at most
  960 pixels wide (default 480, never enlarged) and 5 to 20 frames per second (default 12). Frames
  are decoded, resized, quantised to 256 colours per picture, written and dropped one at a time, so
  memory does not grow with the range. GIF has no sound. This adds the `gif` crate (MIT/Apache-2.0,
  pure Rust, already in the dependency tree through the FilmCraft image stack) as a direct
  dependency with default features off except `std` and `color_quant`.
- **`clip.merge`** joins two to sixteen clips, in order, into one MP4 by copying samples. Every clip
  must have the same tracks with identical codec configuration, timescale and size, as clips
  recorded with unchanged settings do; anything else is refused with a message instead of being
  re-encoded. One edit per clip starts that clip at its own frame-accurate start, so a clip that was
  itself cut between keyframes joins correctly. Picture and sound are placed by their own edits and
  may differ by under one AAC unit (about 23 ms) per join.
- **`clip.normalize`** evens out loudness. The picture is copied; the sound is decoded, measured
  as the sum of all tracks with the FilmCraft BS.1770 meter (gated integrated loudness), scaled by
  one gain so that the programme reaches the target (default -16 LUFS, -30 to -8 accepted), and
  encoded again as AAC with the FilmCraft encoder. All tracks receive the same gain, so game sound
  and microphone keep their balance. A gain that would raise the true peak above -1 dBTP is held
  back and reported, so the result never clips. Gain is limited to 30 dB either way, and a clip
  that is silent or whose sound is not AAC is refused. Memory is bounded: sound is decoded twice,
  once to measure and once to encode, and only the small compressed units are kept.

## Limits and non-goals

No upload, link, account or Discord integration. No loudness normalisation of MKV/WebM sources
that are not AAC, no per-track targets, no true-peak limiter (gain is held back instead), no
re-encoding merge for clips with different settings, no GIF dithering options or palette sharing
across frames, no sound in GIFs. Merging is only offered in the library for clips on disk.

## Consequences

The three paths reuse the pinned FilmCraft decoder, encoder, muxer and meter (ADR 0004); the only
new third-party crate is `gif`. Output quality of the GIF is that of per-frame NeuQuant
quantisation, which can shimmer on gradients. Evidence and timing are in `docs/testing.md`.
