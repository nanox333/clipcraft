# Windows generated-workload capture evidence

Measured 2026-10-21 on Windows NT 10.0.26300, AMD Ryzen 7 5700X3D, AMD Radeon
RX 7900 GRE, driver 32.0.32015.7029. A Parsec virtual display adapter is also present.
These are generated desktop-window measurements, not real-game FPS evidence or vendor-wide support.

## Implementation and limits

Windows Graphics Capture now offers bounded owned D3D11 BGRA texture leases. The Media Foundation
encoder can share that device and convert BGRA to limited-range BT.709 NV12 with a D3D11 video
processor. Capture holds at most four surfaces / 128 MiB; the encoder sample allocator holds at
most eight NV12 surfaces / 128 MiB. Leases prevent overwrite while in use. Pool exhaustion drops
an incoming frame. The recorder also caps hardware pending timestamps at 32 and falls back if an
encoder accepts input without output indefinitely. See [ADR 0014](adr/0014-windows-d3d11-capture-encode.md).
Native capture declines sizes where its budget fits fewer than two surfaces, because retaining
the latest frame would otherwise prevent a one-slot pool from ever receiving the next picture.

Hardware drain errors now propagate to the recorder's final failed status, including after `halt`,
instead of being discarded. A regression test verifies that a previously saved clip stays unchanged.

The native route currently requires SDR BGRA and matching capture/output dimensions with original
framing. Disable HDR compatibility to request BGRA. HDR conversion, resize/crop, incompatible
encoders and screenshots use CPU readback. The default HDR-compatible setting still uses the
existing CPU route. NVIDIA's separately selected NVENC adapter does not yet accept these surfaces;
only the AMD Media Foundation combination below has runtime evidence. No game injection or driver
installation is involved.

Media Foundation input events now wake a bounded callback queue instead of polling. The callback
finishes EndGetEvent itself, resolves an agile COM reference on its callback thread, queues only
plain event IDs, and has a weak reference to the generator owner to avoid a reference cycle.
Event overflow, stalled input and drain timeout return
errors. Low-latency properties use their documented boolean type; H.264 level describes the
requested macroblock rate (4.2 for 1080p60).

## Matched short comparison

Release example `apps/clipcraft/examples/capture_bench.rs` draws 2,048 moving colored rectangles
in its own 1920×1080 egui window. It captures only that process's window. Both runs request
1080p60 H.264 CBR 12 Mbps, Fast, two-second GOP, ten-second replay capped at 32 MiB,
and a generated 48 kHz stereo 440 Hz AAC track. Warm-up is three seconds, followed by 15 seconds
of recording. `cpu` forces CPU readback and RGBA input with the same capture/encoder selection.

| Measurement | CPU input | Native D3D11 input |
| --- | --- | --- |
| Captured / encoded / dropped | 826 / 473 / 351 | 827 / 474 / 352 |
| Mean conversion + encode call, including wait | 31.307 ms | 31.701 ms |
| Full-frame CPU readbacks | 828 | 0 |
| CPU / native encoder submissions | 476 / 0 | 0 / 477 |
| Workload frame time p50 | 6.039 ms | 5.997 ms |
| Workload frame time p95 | 8.463 ms | 7.919 ms |
| Workload frame time p99 | 10.211 ms | 8.972 ms |
| Mean CPU, percent of one logical core | 105.88% | 36.21% |
| Mean resident memory | 216.27 MiB | 163.97 MiB |

These are single paired runs with no statistical confidence claim. Frame-time values are intervals
between egui updates, not GPU timestamps or independently measured
display presents. This colored-rectangle workload is light on this GPU; it does not establish
behavior under GPU saturation. A heavier repeatable workload and present-level timing remain open.
Processor and resident-memory
samples are emitted once per second; CPU percentage is percent of one logical core and includes
the generated egui workload. Table means use the 12 samples at elapsed times 5–17 seconds in each
run. The corrected agile-reference build averaged 45.18% of one core and 164.54 MiB in that window;
single-run variation and this reference change require repeated pairs before a percentage claim.
The native run's late RSS samples were about 162–170 MiB versus
216–224 MiB for CPU input. Neither path sustained 60 encoded fps: both produced about 31.6 fps.
The native run spent 14.47 seconds waiting for Media Foundation input events, compared with
0.097 seconds in GPU conversion. Removing readback did not resolve encoder throughput. The cause
has not been isolated; do not attribute it to a particular driver or display setup without proof.
Power, actual game impact and repeated paired-run spread remain unmeasured. Five one-second
Windows GPU Engine samples during the 30 fps soak reported 2.94–2.97% on the process's 3D engine
and 7.53–8.16% on its video-codec engine. These include the generated workload and are a short
busy-system sample, not a matched CPU/native GPU-use comparison.

Earlier archived runs without audio are retained locally but are not matched comparisons.
One preliminary 30 fps run overlapped compilation and is excluded from performance comparisons.
Raw logs and generated media are ignored under `target/windows-capture-evidence`.

## Reproduction and soak

```text
cargo build --release -p clipcraft --example capture_bench
target/release/examples/capture_bench.exe idle 15 target/idle.mp4 60
target/release/examples/capture_bench.exe cpu 15 target/cpu.mp4 60
target/release/examples/capture_bench.exe native 15 target/native.mp4 60
target/release/examples/capture_bench.exe native 7200 target/soak.mp4 30
cargo test -p clipcraft-platform --lib gpu_leases_preserve_pixels -- --ignored --test-threads=1
```

Keep other GPU workloads and builds idle for timing comparisons. The benchmark validates argument
bounds, fails on recorder error, ten seconds without encoder progress, replay above 32 MiB or
process RSS above 512 MiB. It saves a five-second checkpoint every five minutes and a final clip
without overwriting existing outputs. Frame-time samples are capped at four million (32 MB).
Checkpoint output is bounded by the maximum two-hour run (at most 24 checkpoints plus the final clip).
Inspect every clip with an independent decoder. A successful process exit is a reliability result,
not a 60 fps performance pass: dropped frames and actual throughput must be assessed separately.

A preliminary 1080p30 soak was stopped after 378 seconds to incorporate the explicit agile COM
reference fix. At 300 seconds it had encoded 8,981 frames, dropped 11, restarted zero times and
held 17,045,644 replay bytes; its checkpoint decoded without error. That interrupted run is not
a two-hour pass. Short comparison results above predate this thread-reference correction.
The corrected build is running its own two-hour soak (`soak-agile-1080p30.txt`); completion remains
open. A corrected-build 15-second 60 fps check encoded 473 frames, dropped 353, used zero CPU
readbacks and reported workload p99 8.896 ms, consistent with the earlier throughput limitation.
At its first five-minute checkpoint the corrected soak had encoded 8,992 frames, dropped eight,
restarted zero times and held 17,587,985 replay bytes.
At ten minutes it had encoded 17,992 frames with eight drops and zero restarts; both five- and
ten-minute checkpoint clips decoded without error. The running binary's SHA-256 is
`08be09d85c1eaf967a2760c593d3f7fb7265a4f1d1ed71632a05a698af4ac336`.
It predates the recorder drain-error propagation change and the rejection of resolutions that
would leave a one-slot capture pool. Both have regression coverage; neither changes this 1080p
capture/conversion/event-delivery path.
CI and integration checks run concurrently,
so soak timings are not an unloaded performance comparison.
The explicit D3D11 integration test passed on this AMD device, including black/white/RGB/gray NV12
agreement with the portable BT.709 converter within three code values, held-pixel preservation,
pool exhaustion/reuse and resize. `ffprobe` identifies the matched native clip as 1920×1080 H.264,
limited-range BT.709 plus AAC; `ffmpeg` decodes the complete file without error.

Device removal, exclusive fullscreen, protected
content, cursor/HDR parity, real microphones/speakers, hotkey integration and other GPUs remain
open release checks. Deterministic tests cover hostile texture descriptions, event-queue bounds,
event ownership cleanup and silent-encoder fallback. The explicit D3D11 test exercises actual
pixel readback, held leases, exhaustion/reuse and old-size leases surviving resize.

## Verification

The reviewed source passed `cargo xtask ci` (workspace/all-target tests, Clippy with warnings denied,
layer and asset checks, and dependency license/source/ban checks) and
`cargo xtask docs-check origin/main`. Tests ran serially with a workspace-local temporary directory;
the pinned `cargo-deny` binary is under ignored `target/performance-tools`. The full gate log is
`target/windows-performance-ci-reviewed.log`. The real D3D11 test also passed when explicitly run
on this device. No two-hour, real-game, device-removal or other-vendor pass is implied.
