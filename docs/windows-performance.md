# Windows generated-workload capture evidence

Initial measurements 2026-10-21; follow-up verified 2026-10-23 on Windows NT 10.0.26300,
AMD Ryzen 7 5700X3D, AMD Radeon
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

The native route requires SDR BGRA, which is now the default (HDR compatibility is off unless
turned on). A different output size, crop or letterbox is applied on the graphics card by the D3D11
video processor (source and destination rectangles, black background) when the placement sits on
even pixels; otherwise, and for HDR conversion, incompatible encoders and screenshots, the CPU
route is used. The GPU scaling has a Windows test (`d3d11` module, a scaled and letterboxed 32x32
surface) but no real-GPU measurement yet. Turning HDR compatibility on still uses the CPU route. NVIDIA's separately selected NVENC adapter does not yet accept these surfaces;
only the AMD Media Foundation combination below has runtime evidence. No game injection or driver
installation is involved.

Media Foundation input events now wake a bounded callback queue instead of polling. The callback
finishes EndGetEvent itself, resolves an agile COM reference on its callback thread, queues only
plain event IDs, and has a weak reference to the generator owner to avoid a reference cycle.
Event overflow, stalled input and drain timeout return
errors. Low-latency properties use their documented boolean type; H.264 level describes the
requested macroblock rate (4.2 for 1080p60).

## Historical matched short comparison (2026-10-21)

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
has not been isolated; it did not reproduce in the follow-up controls, before the cadence changes
below. Do not attribute that historical stall to a particular driver or display setup without proof.
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
cargo build --release -p clipcraft-platform --example encoder_bench
target/release/examples/encoder_bench.exe 60 240 target/encoder-60.mp4 unpaced
target/release/examples/encoder_bench.exe 60 240 target/encoder-60-paced.mp4 paced
target/release/examples/capture_bench.exe idle 15 target/idle.mp4 60
target/release/examples/capture_bench.exe cpu 15 target/cpu.mp4 60
target/release/examples/capture_bench.exe native 15 target/native.mp4 60
target/release/examples/capture_bench.exe native 60 target/native-minute.mp4 60
target/release/examples/capture_bench.exe headless-silent 10 target/headless.mp4 60
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

Diagnostic modes also include `native-silent` (WGC without audio), `synthetic` /
`synthetic-silent` (generated CPU frames with the egui workload), and `headless` /
`headless-silent` (generated CPU frames without a window). The generated audio is a tone, not a
microphone or system device. WGC and MF metrics are marked `n/a` when that seam is not applicable.
Final cadence uses measured feed time, halts before the final save, and includes actual MF output
through drain. Counters are process-local; fresh reads mean a newly observed WGC sequence, not
independently verified distinct pixels or display presents. Priming and an in-flight capture can
add a few reads around the interval boundary. The encoder-only tool caps submissions at 600,
pending timestamps at 32 and encoded output at 128 MiB, verifies every returned picture through
drain, and checks a 60-second feed/drain budget between backend calls. This is not a hard process
timeout around arbitrary driver calls. Its mostly static gradient differs from the animated WGC
workload; throughput there cannot establish native capture or real-game performance.

A preliminary 1080p30 soak was stopped after 378 seconds to incorporate the explicit agile COM
reference fix. At 300 seconds it had encoded 8,981 frames, dropped 11, restarted zero times and
held 17,045,644 replay bytes; its checkpoint decoded without error. That interrupted run is not
a two-hour pass. Short comparison results above predate this thread-reference correction.
The corrected build completed the full two-hour **1080p30** soak with exit code zero. Results were
verified on 2026-10-22 from `soak-agile-1080p30.txt` and the saved media:

| Measurement | Result |
| --- | --- |
| Captured / encoded / dropped | 216,000 / 215,992 / 8 (0.0037%) |
| Restarts / capture pool drops | 0 / 0 |
| CPU readbacks / CPU encoder inputs | 0 / 0 |
| Native encoder submissions | 215,993 (final pipeline counter; recording status was sampled before stop) |
| Peak / final sampled RSS | 177.77 / 118.62 MiB |
| Mean sampled CPU, including generated workload | 33.89% of one logical core |
| Peak sampled replay payload | 17,684,494 bytes (32 MiB budget) |
| Saved and independently decoded files | 23 five-minute checkpoints plus final clip: 24 / 24 passed |
| Largest audio/video duration difference | 14 ms; this does not measure perceptual sync |
| egui update interval p50 / p95 / p99 | 6.059 / 7.233 / 8.156 ms, 1,187,240 samples |

Every saved file contains 1920×1080 H.264 plus AAC and passed complete `ffmpeg -v error -xerror`
decoding; `ffprobe` checked stream layouts and durations. This proves periodic replay saves over
two hours, not a continuous two-hour output file. Resource statistics use 7,177 one-second samples;
sampling is approximate. The running binary's SHA-256 was
`08be09d85c1eaf967a2760c593d3f7fb7265a4f1d1ed71632a05a698af4ac336`.
It predates the recorder drain-error propagation change and the rejection of resolutions that
would leave a one-slot capture pool. Both have regression coverage; neither changes this 1080p
capture/conversion/event-delivery path.
CI and integration checks ran concurrently during part of the soak, so its timings are not an
unloaded performance comparison. A corrected-build 15-second 60 fps check encoded 473 frames,
dropped 353, used zero CPU readbacks and reported update-interval p99 8.896 ms. The 1080p30
reliability pass does not prove two-hour 1080p60 stability; the later short throughput checks are
recorded separately below.
The explicit D3D11 integration test passed on this AMD device, including black/white/RGB/gray NV12
agreement with the portable BT.709 converter within three code values, held-pixel preservation,
pool exhaustion/reuse and resize. `ffprobe` identifies the matched native clip as 1920×1080 H.264,
limited-range BT.709 plus AAC; `ffmpeg` decodes the complete file without error.

Device removal, exclusive fullscreen, protected
content, cursor/HDR parity, real microphones/speakers, hotkey integration and other GPUs remain
open release checks. Deterministic tests cover hostile texture descriptions, event-queue bounds,
event ownership cleanup and silent-encoder fallback. The explicit D3D11 test exercises actual
pixel readback, held leases, exhaustion/reuse and old-size leases surviving resize.

## 1080p60 follow-up (verified 2026-10-23)

The source was advanced to upstream `dfe99f3` before these changes. The historical 32 fps MF
input-wait stall no longer reproduced **before any cadence fix**: an upstream WGC run encoded
814 pictures in 15 seconds and spent only 7.067 ms waiting for MF input. Its remaining roughly
54 fps cadence was different from the historical encoder stall. The original stall remains
unexplained; no driver, power-setting or universal encoder fix is claimed.

Direct generated CPU RGBA input isolated the encoder from WGC, recorder, UI and audio:

| Encoder-only mode | Submitted / returned / saved | Throughput | Mean / p95 encode call |
| --- | --- | --- | --- |
| Unpaced control | 240 / 240 / 240 | 157.588 pictures/s | 6.316 / 6.945 ms |
| Final paced 60 fps | 240 / 240 / 240 | 60.090 pictures/s | 4.149 / 4.323 ms |

Both independently decoded to 240 H.264 pictures over four seconds at 60/1. Their input/output
media types reported 60/1; the driver advertised 1,944,000 macroblocks/s. These results rule out
a universal 32 fps encoder cap in this session, not every content or native-resource bottleneck.

Returning the latest cached WGC picture immediately restored recorder output pacing, but still
repeated pictures when WGC arrived at about 55/s. The same generated-window workload confirmed
this in a fresh control. Requesting half the output frame period as the SDR minimum update
interval provided delivery headroom:

| Nominal 15-second native run | Fresh WGC reads | Repeated reads | GPU copies | CPU readbacks |
| --- | --- | --- | --- | --- |
| Latest cache, default OS interval | 828 | 76 | 829 | 0 |
| Latest cache, half-period interval | 903 | 1 | 1,243 | 0 |

These short controls include priming/boundary reads and used the earlier pre-stop status report;
they are cadence evidence, not precise latency or energy comparisons. More GPU copies are an
explicit tradeoff for fresh delivery. The setting is an OS eligibility throttle, not a periodic
delivery guarantee. Unsupported interval requests retry the default only for that specific API
error. HDR keeps the default interval; CPU callback readbacks are admitted at most once per
absolute output-rate slot. These runs were made when HDR compatibility defaulted to on; it now
defaults to off, so they describe the default path (settings saved earlier keep their value).

The final native run halted before the final save, measured its feed interval and counted actual
MF output through drain. It requested 1080p60 with generated AAC tone audio:

| Measurement | Result |
| --- | --- |
| Measured feed interval / expected pictures | 60.026087 s / 3,601 |
| Captured / encoded and returned / dropped | 3,602 / 3,589 / 13 (0.361% of captured) |
| Returned-picture deficit against expected | 12 (0.333%); 59.791 pictures/s including startup |
| Fresh WGC reads / repeated reads | 3,602 / 0; 60.007 fresh reads/s |
| CPU readbacks / CPU encoder inputs / pool drops / restarts | 0 / 0 / 0 / 0 |
| Native inputs / GPU copies | 3,589 / 4,953 |
| Mean conversion + encode call / total MF input wait | 0.504 ms / 12.676 ms |
| Peak / final sampled RSS | 177.10 / 170.96 MiB |
| Mean sampled CPU / RSS, including workload and warm-up | 31.49% of one logical core / 168.60 MiB; 62 samples |
| Largest ten-second health-sampled replay payload | 17,721,600 bytes; 32 MiB enforced budget |
| egui update interval p50 / p95 / p99 | 6.062 / 6.672 / 7.509 ms; 9,931 samples |
| Saved replay independently decoded | 343 H.264 pictures, 5.716667 s at 60/1, plus AAC |
| Audio/video stream-duration difference | 13.333 ms; not perceptual sync evidence |

All 13 drops were already present at the ten-second health check and did not grow. Both fresh-read
and returned-picture deficits are under 1%, with bounded replay/RSS and no readbacks. This closes
the short generated-workload throughput criterion on this AMD SDR configuration. It is not a
continuous one-minute saved file, a two-hour 60 fps soak, real-game FPS evidence or a statistical
CPU/GPU/power comparison. No builds or other test jobs ran during the timed follow-up runs.

The CPU-input fallback also saved and fully decoded a 15-second generated-window run: 902 captured,
888 returned, 14 startup drops, 902 readbacks, no restarts, mean encode call 5.495 ms. It is a
fallback smoke check, not the minute-long throughput gate. Final paced encoder, headless-silent
diagnostics, controls and native runs produced 12 generated MP4 files; all passed complete
`ffmpeg -v error -xerror` decoding and `ffprobe` stream/frame checks. Raw logs, probes and media
are ignored under `target/windows-60fps-evidence`. The final benchmark binary SHA-256 is
`ae1081ee870fd5e3b48bc560c8e176cbf2d1a9a90dad7925ba078087f75aeb5f`.

Negotiation diagnostics reported zero stream changes in these runs. Deterministic tests cover
exact rational rates, hostile/partial types, preserving the accepted contract over advertised
defaults, changed SPS/PPS rejection, sample ownership and unsupported caller alignment. Actual
driver-triggered renegotiation and old-Windows unsupported-interval fallback remain unforced
hardware checks. Previously saved clips remain untouched by failures; the existing running
hardware-fallback policy still discards unsaved replay when starting a new software stream.

## Verification

The earlier 1080p30 source passed `cargo xtask ci` (workspace/all-target tests, Clippy with warnings denied,
layer and asset checks, and dependency license/source/ban checks) and
`cargo xtask docs-check origin/main`. Tests ran serially with a workspace-local temporary directory;
the pinned `cargo-deny` binary is under ignored `target/performance-tools`. The full gate log is
`target/windows-performance-ci-reviewed.log`. The real D3D11 test also passed when explicitly run
on this device. That two-hour pass is limited to generated-window 1080p30; the separate one-minute
1080p60 result above does not imply a real-game, device-removal or other-vendor pass.

After integrating upstream `f12db5f`, the desktop and CLI release builds, `cargo xtask ci` and
`cargo xtask docs-check origin/main` passed. The complete merged gate log is
`target/windows-60fps-latest-ci.log`. Windows CI exposed a recovery fixture using ordinary paths
instead of the canonical paths stored by real editor imports; the fixture now preserves that
contract and all missing-file/probe assertions pass. The timed results above used the earlier
`dfe99f3` base plus these capture changes; the later upstream update leaves the measured
WGC/D3D11/MF implementation unchanged. It adds no new soak or game-performance evidence.
