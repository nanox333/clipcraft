# Where this encoder comes from

This crate is a copy of `filmcraft-h264enc` (and its tiny dependency `filmcraft-bitstream`) from
FilmCraft, <https://github.com/storytold/filmcraft>, commit
`6c3948a36c80e4996d924766ba7157eec4fca85e`, directories `crates/h264enc` and `crates/bitstream`.
FilmCraft is dual-licensed MIT OR Apache-2.0; both licence texts are in this folder and the
copyright line is kept. It is a clean-room encoder written from the public H.264 specification.

ClipCraft now maintains its own copy so the encoder can be tuned for screen recording. The
workspace `[patch]` section makes every FilmCraft crate that uses it (the export crate, for
example) use this copy too, so there is one encoder in the build.

The upstream README is kept as `UPSTREAM-README.md`. Upstream's own test files (an ffmpeg oracle
suite that needs FilmCraft's private `testkit` crate) were not copied; ClipCraft's tests (and the
unit tests inside `src/`) cover the encoder through the recorder and exporter, and
`crates/clipcraft-record/examples/encoder_profile.rs` measures it.

## Changes from upstream

Every change so far is speed-only: the bitstream is byte-identical to upstream's, checked with
`cargo run --release -p clipcraft-record --example encoder_hash W H FILE` (a hash of all output for 8
settings, before and after). List each change here with the reason and its measurement, so a later
merge from FilmCraft stays possible:

| Date | Change | Why | Measured |
| --- | --- | --- | --- |
| 2026-10-28 | P_Skip is decided without running the transforms when the residual provably quantises to nothing (`transform::ZeroBounds`, `slice::residual_is_zero`) | Profiling showed 45-65% of encode time was spent proving that unchanged macroblocks have no residual | 1080p, 1 thread, Speed, QP 28: 28.0 -> 21 ms on a scrolling game UI clip, 20.9 -> about 14 ms on a still picture. Output identical. |
| 2026-10-28 | Blocks too quiet to leave a coefficient skip their transform and quantiser inside `code_luma8`, `code_luma4` and `code_chroma` | Same bound applied per block, for macroblocks that are only partly quiet | With the row above: game 28.0 -> 18.1 ms (-35%), IDE desktop 34.9 -> 26.4 ms (-24%), still 20.9 -> 11.9 ms (-43%). In the recorder's own setting (VBR 12 Mbit/s, adaptive quantisation, 2 threads): game 43.1 -> 36.6 ms, desktop 38.7 -> 32.2 ms (-15% to -17%), because lower QPs leave fewer quiet blocks. Output identical. |
| 2026-10-28 | Deblocking skips the inside of a P_Skip macroblock and decides its outer edges from one motion vector | Every such edge has strength 0 or depends on that single vector, but strength was worked out per 4-sample segment | About 1 ms per 1080p frame on mostly static pictures. Output identical. |
