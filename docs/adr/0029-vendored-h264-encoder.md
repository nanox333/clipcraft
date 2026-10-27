# ADR 0029: Keep the H.264 encoder in this repository

Status: accepted, 2026-10-26.

## Context

ClipCraft's software H.264 encoder was `filmcraft-h264enc`, a git dependency on FilmCraft pinned to
one commit. On a machine without a hardware encoder it decides how many pictures per second a clip
can have (about 53 at 1440x900 on two threads of a 4-core computer, measured with
`crates/clipcraft-record/examples/encoder_profile.rs`). Improving it for screen recording (screen
content, low latency, no B-frames, bounded threads) meant changing code in another repository.

## Decision

* Copy `filmcraft-h264enc` and its one dependency `filmcraft-bitstream` into `crates/` at the pinned
  commit, keeping the MIT and Apache-2.0 licence texts and copyright line (the source repository is
  MIT OR Apache-2.0).
* Use a workspace `[patch]` for the FilmCraft git source so FilmCraft's own crates that depend on the
  encoder use this copy: the build has exactly one encoder.
* The copy keeps FilmCraft's lint settings (`unsafe` forbidden, the same clippy allowances); ClipCraft's
  stricter style lints can be adopted gradually. The elided-lifetime idiom was fixed (mechanical).
* Upstream's ffmpeg-oracle tests were not copied because they need FilmCraft's private `testkit` crate.
  The unit tests in `src/` came along, and ClipCraft's recorder and exporter tests exercise the
  encoder. Add oracle tests here when the encoder is changed.
* Every change from upstream is listed with its reason and measurement in
  `crates/filmcraft-h264enc/UPSTREAM.md`, so FilmCraft fixes can still be merged by hand.

## Consequences

ClipCraft owns the encoder's maintenance and correctness. FilmCraft improvements no longer arrive
automatically. The encoder crates are in the workspace gate (formatting, clippy, tests, licences,
layers). No behaviour changed with this ADR: the first commit is a byte-identical copy apart from
lifetime annotations.
