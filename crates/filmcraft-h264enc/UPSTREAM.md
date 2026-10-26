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

None yet. List each change here with the reason and its measurement, so a later merge from
FilmCraft stays possible:

| Date | Change | Why | Measured |
| --- | --- | --- | --- |
