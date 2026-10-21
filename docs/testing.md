# Testing

The test strategy treats capture, media, and local library metadata as untrusted and checks failure
behavior as carefully as the happy path. Capture/replay, playback, persistent library metadata,
audio, and trim export have deterministic tests; platform evidence and real-device limits are
recorded in [`gaps.md`](gaps.md) and [`release-checklist.md`](release-checklist.md).

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

## Documentation gate

Documentation ships in the same pull request as implementation. For any changes under `apps/`,
`crates/`, or `tools/` (or workspace dependency manifests), update `README.md` and `docs/gaps.md`;
update the matching backlog and roadmap when a status changes, and update architecture/testing/CLI/
release/ADR guidance as needed. Pull-request CI runs `cargo xtask docs-check origin/<base-branch>`
and fails when an implementation diff does not include both `README.md` and `docs/gaps.md`. The
same check runs on direct pushes to `main`, against the previous commit.
Both workflow steps explicitly use Bash on Linux and Windows so `$BASE_REF` expands consistently.

To run the same check locally, first fetch the target branch and pass its remote-tracking name:

```sh
git fetch origin main
cargo xtask docs-check origin/main
```

The check uses the merge base with the supplied ref and includes committed, staged, unstaged, and
untracked local changes. Documentation-only changes pass without requiring README or gap-report
edits; implementation changes require both.

## Required evidence by area

The offline CPU benchmark is `cargo run --release -p clipcraft-record --example performance`.
It measures patterned-frame conversion, a 60-second encoded-video snapshot and long-GOP replay
insertion. See [performance.md](performance.md) for the machine, method, measured baseline and
remaining real-device checks. Timing is informational and is not a flaky CI assertion.

- **Time/library metadata:** property tests for rational conversions and trim algebra; round-trip,
  migration, truncation, oversized values, malformed metadata, missing media, and atomic-write tests.
- **Replay/capture:** deterministic fake-provider tests for ring eviction, limits, timestamps,
  post-roll, dropped frames, save races, start/stop, device loss, and full disk. Real capture tests
  run on platform hardware and state their OS/device matrix.
- **Low-copy GPU capture (Windows SDR partial):** test bounded surface leases, device loss, format/color
  conversion, and parity with the CPU-frame fallback. A fake native surface and hardware encoder
  already verify that compatible surfaces bypass readback and unsupported encoders use the fallback;
  Windows WGC/D3D11 and AMD Media Foundation also have generated-window evidence and an explicit
  real-device lease/readback/resize test. Reproduction and the checkpointed Windows soak example
  are in [windows-performance.md](windows-performance.md). Other GPU/format pairs remain unrun.
  Real-device evidence names the GPU,
  driver, OS, compositor, capture/encoder APIs, resolution/FPS, CPU use, GPU use, dropped frames, and
  game frame-time delta. Never infer sub-1% CPU use or broad compositor support from a compile or
  mock.
- **Game-only capture:** engine tests cover process match priority, exact Steam IDs, duplicate
  observations, process exit/restart, and manual pause. The X11 adapter uses a synthetic window
  with a published `_NET_WM_PID`; Windows still needs a real-device run, and Wayland app-to-window
  matching is unsupported. Verify that empty allowlists never scan or start capture and that the
  process table is polled only while Game-only mode is enabled.
- **Wayland:** a stand-in desktop on a private D-Bus session bus answers the ScreenCast portal;
  a real PipeWire server carries a real video node (GStreamer test source); format pods are
  compared with PipeWire's own builder; C structure layouts are asserted against the headers.
  Tests skip, saying why, without those tools. Tests against a real portal backend are
  `#[ignore]`d. See [wayland.md](wayland.md).
- **Audio:** generated clocked sources verify sync drift, mute/gain, resampling, microphone loss,
  and recovery over long runs.
- **Export:** `cargo test -p clipcraft-export` includes Rust-generated H.264/AAC MP4/MKV fixtures
  for original-quality trim and estimated-size compression. Compression tests decode the output,
  check first-picture content, 60-to-30 fps conversion, duration, separate audio tracks and click
  alignment, actual-size reporting, progress, malformed inputs, cancellation, temporary-file
  collisions, and a destination created while encoding. FFmpeg additionally checks full-file
  decoding when installed, solely as an external test oracle. Native preview screenshots and
  keyboard checks exercise the selector, custom size, and completion message. Software throughput
  measurements on 30/60-second clips are recorded separately from real-GPU capture evidence.
- **UI:** test commands/state transitions independently of widgets. For visible changes, run the
  desktop build, inspect keyboard navigation at a representative display scale, and attach a
  before/after screenshot using generated/openly licensed media.
- **CLI:** test every command's schema, invalid parameters, local path behavior, and JSON output.
  CLI tests must not start a network or control server.
- **Privacy/safety:** assert that core workflows work offline, no data leaves the device by default,
  logs are redacted, memory/disk use is bounded, deletion is safe, and source media is preserved.
- **Encrypted backup (when implemented):** use a fake provider to assert opt-in behavior, ciphertext-
  only transfer, integrity checks, retry/cancel, fresh-install restore, and documented key-loss
  behavior; test a real provider only with disposable credentials and synthetic clips.

Use synthetic fixtures in tests. Large real-media corpora are pinned by commit and SHA-256 and
fetched into ignored cache directories; never commit captured gameplay or private user media.

## Software export sample measurements

On 2026-10-21, the optimized dependency/debug app build exported generated `testsrc2` H.264/AAC
MP4 inputs at 320×180/60 fps in this shared cloud container. Output was 320×180/30 fps with copied
AAC. Each job ran once; wall time includes opening, decoding, encoding, and file publication.
The source files were 7.28 MB (30 s) and 14.56 MB (60 s). These low-resolution synthetic samples
verify offline behavior and target response; they do not establish gaming-PC or capture performance.

| Input duration | Target | Actual output | Wall time |
| --- | --- | --- | --- |
| 30 s | 10 MB | 7.26 MB | 3.81 s |
| 30 s | 25 MB | 7.26 MB | 2.88 s |
| 30 s | Custom 5 MB | 4.46 MB | 2.87 s |
| 60 s | 10 MB | 8.98 MB | 5.74 s |
| 60 s | 25 MB | 14.53 MB | 10.20 s |
| 60 s | Custom 5 MB | 4.48 MB | 9.90 s |

The larger targets do not force a small source to fill the budget. A separate generated full-HD
test exercises the actual 1920×1080-to-1280×720 resize and decoding path. Reproduce timing with
`time -p clipcraft-cli clip export INPUT OUTPUT 0 DURATION TARGET_MB`; FFmpeg may generate the
input fixture but is never called by the export command. Outputs, timing reports, and UI
before/after captures belong in ignored local artifact folders.

The native app was also opened at 1440×900 against a virtual X display. Exporting an 8-second
generated clip through Trim → Custom size → 1 MB wrote 844,672 bytes from a 2,913,501-byte source;
the source SHA-256 stayed unchanged and the completion message displayed the actual size and
target. T opens trim, I/O mark boundaries, text entry changes the custom target, and playback
shortcuts are suppressed while the export popup is open. A regression test covers that popup
conflict, and UI smoke tests draw all size choices at narrow and desktop widths.
