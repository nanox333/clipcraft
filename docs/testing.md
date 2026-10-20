# Testing

The test strategy treats capture, media, and local library metadata as untrusted and checks failure
behavior as carefully as the happy path. The Rust workspace and first checks are available; capture,
playback, persistent metadata, and export coverage remain future work.

## Canonical gate

Run `cargo xtask ci` before completing an issue. CI calls the same command. It includes formatting,
strict Clippy, workspace tests, layer validation, asset attribution checks, and a `cargo-deny`
license audit. Keep focused test commands in crate docs, but do not create a second gate with
different pass criteria.

Current focused checks include `cargo test -p clipcraft-time` for time math, `cargo test -p clipcraft-media` for container probing
including byte-corruption and oversized-header cases, `cargo test -p clipcraft-player` for playback, sound, speed, stepping, looping, track selection, stalled devices, and thumbnails (it encodes its own H.264 clips in pure Rust, so it needs no
external tools), `cargo test -p clipcraft-frame` for queue limits,
`cargo test -p clipcraft-engine` for bounded local scans and command dispatch, and
`cargo test -p clipcraft` for UI helpers, keyboard handling, and smoke tests that draw every screen
(library, empty states, details, settings, the player windowed and fullscreen) at window sizes from
1×1 to 6000×4000 with awkward titles, so a layout that panics fails the gate.

Long-run and release evidence is described in [`release-checklist.md`](release-checklist.md):
`cargo run --release -p clipcraft-record --example soak -- --minutes N [--freeze-for S]` measures
memory, processor time, dropped pictures, and clip validity over time, and can freeze the recorder
to simulate sleep.

## Required evidence by area

- **Time/library metadata:** property tests for rational conversions and trim algebra; round-trip,
  migration, truncation, oversized values, malformed metadata, missing media, and atomic-write tests.
- **Replay/capture:** deterministic fake-provider tests for ring eviction, limits, timestamps,
  post-roll, dropped frames, save races, start/stop, device loss, and full disk. Real capture tests
  run on platform hardware and state their OS/device matrix.
- **Wayland:** a stand-in desktop on a private D-Bus session bus answers the ScreenCast portal;
  a real PipeWire server carries a real video node (GStreamer test source); format pods are
  compared with PipeWire's own builder; C structure layouts are asserted against the headers.
  Tests skip, saying why, without those tools. Tests against a real portal backend are
  `#[ignore]`d. See [wayland.md](wayland.md).
- **Audio:** generated clocked sources verify sync drift, mute/gain, resampling, microphone loss,
  and recovery over long runs.
- **Export:** generated clips verify cut/trim boundaries, audio sync, container/codec metadata,
  cancellation, cleanup, and no overwrite.
- **UI:** test commands/state transitions independently of widgets. For visible changes, run the
  desktop build, inspect keyboard navigation at a representative display scale, and attach a
  before/after screenshot using generated/openly licensed media.
- **CLI:** test every command's schema, invalid parameters, local path behavior, and JSON output.
  CLI tests must not start a network or control server.
- **Privacy/safety:** assert offline operation, local-only writes, redacted logs, bounded memory and
  disk use, safe deletion, and no changes to source media.

Use synthetic fixtures in tests. Large real-media corpora are pinned by commit and SHA-256 and
fetched into ignored cache directories; never commit captured gameplay or private user media.
