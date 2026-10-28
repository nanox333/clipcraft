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

All changes are for the Speed preset (what recording uses) unless noted; none changes the stream format. List each change here with the reason and its measurement, so a later merge from
FilmCraft stays possible:

| Date | Change | Why | Measured |
| --- | --- | --- | --- |
| 2026-10-26 | Static-content skip: Speed preset marks macroblocks identical to the previous source picture and codes them as P_Skip without searching | Desktop recordings are mostly unchanged | desktop clip 23.1 -> 10.9 ms/picture (QP26, 2 threads); output size and PSNR unchanged |
| 2026-10-28 | Speed preset turns on inter partitions and a finer motion search (range 16, one sub-pel round); 8x8 intra and adaptive 8x8 transform stay off | Desktop clips spent 1.5x the bits of Balanced for no reason: partitions were the missing piece | IDE desktop clip, QP 28, 1 thread: 2.00 -> 1.32 MB (-34%), PSNR 42.51 -> 42.44 dB, 20 -> 31 ms/picture; game UI clip: same size, 7.3 -> 8.1 ms; 8x8 intra and adaptive transform added no saving |
| 2026-10-26 | Zero-residual shortcut: skip transform+quantise for blocks that provably quantise to zero (Cauchy-Schwarz bound on SSD) | Most inter blocks on screen content are all-zero | bit-identical output on all test clips; unit tests incl. adversarial blocks |
| 2026-10-26 | Deblocking skips edges between same-vector P_Skip neighbours | Filter is a no-op there | bit-identical |
| 2026-10-26 | Intra candidate gate in P/B pictures (non-RD): skip intra when its cost is >= 1.25x the best inter cost | Intra rarely wins there | measured, no PSNR change on test clips |
| 2026-10-26 | `fast_intra`: skip diagonal intra 4x4 modes when DC/H/V already fit well | Cuts mode search | keyframes ~30% fewer instructions |
| 2026-10-26 | Straight-line 4x4 SATD, chroma motion-compensation fast paths for whole/half-pel vectors | Hot inner loops | exhaustive/random tests against the reference |

Measured with `encoder_profile` on 1440x900 clips (desktop, scroll, pan, rotate, game-like), ms per
picture, before -> after at 2 threads, QP26: desktop 23.1 -> 10.9, scroll 48.9 -> 37.9, pan 51.5 -> 37.4,
rotate 42.0 -> 27.8, game 55.2 -> 39.6. Rejected (no gain or quality loss): step-1 ME early exit,
quarter-pel off, adaptive sub-pel, vectorised deblock filter.
