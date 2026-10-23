# Performance review

Original recording audit: 2026-10-21. Editor export measurements: 2026-10-23 (M11.1e below).

This review follows the first-party paths across all 13 workspace packages. Findings below are
source-level costs unless a measurement is attached. Dependency codec internals, real game frame
times, GPU utilization, power and storage throughput have not been profiled. There is no evidence
yet for a competitive ranking or an end-to-end capture speed claim.

The original audit and before/after measurements used baseline `995bf26`. The performance commit
was subsequently rebased onto `d189e20`, which adds compressed export, game-only arming and an
opaque native-surface interface. Those new workflows are outside the original timing measurements.
The native handoff has fake-provider tests but still lacks a real platform capture/encoder pair;
the CPU capture/conversion costs described below remain relevant to actual backends.

## M2.P1 — Reduce replay-save stalls and redundant conversion work

**Problem:** saving a clip copies all encoded video under the replay mutex, blocking insertion of
new encoded pictures. Replay insertion repeatedly searches for the next keyframe. RGBA conversion
clears and writes alpha across the whole output before overwriting those bytes again.

**Scope:** `clipcraft-record`, synthetic benchmarks and correctness tests. No format, quality,
framing, capture scheduling, encoder preset, file safety or UI behavior changes. No dependencies.
This is a local work item under M2.2 and the game-performance gap. Estimate: one focused change;
reference-GPU validation is separate work.

**Acceptance:** on the machine below, a 60-second video snapshot takes under 1 ms; full-size 1080p
RGBA conversion with four threads takes under 1 ms; inserting 14,400 samples with 3,600-frame GOPs
takes under 2 ms. Snapshots retain byte-identical payloads after eviction/restart; keyframe selection,
byte limits, malformed-frame rejection, opaque output and letterboxing still pass tests. Run
`cargo xtask ci`. These local timing targets are review criteria, not timing-sensitive CI tests.

### Changes

- Encoded video payloads use immutable `Arc<[u8]>`. Snapshots copy small sample descriptors and
  increment reference counts. The encoder's `Vec` becomes shared storage outside the replay lock.
  Callers constructing `EncodedSample` from a `Vec<u8>` now use `data: bytes.into()`.
  Conversion to `Arc<[u8]>` can copy a packet once on insertion; it avoids copying the entire
  buffered video on every save. AAC payloads are still copied when saving.
- Cache the next GOP boundary; search again only after evicting a group. Insertions no longer
  repeatedly scan the first GOP. `clear` resets the cache. Timestamp spans saturate on extreme
  input instead of overflowing.
- Reuse RGBA output contents when every byte will be overwritten; clear only for black bars.
  Unscaled I420 and RGBA conversion skip allocation and computation of unused interpolation tables.

The replay byte/window limits are unchanged. A save retains the samples it references even after
the ring evicts them; this was also true of the old copied snapshots. The desktop permits one save
job at a time. Direct library callers must bound their own concurrent saves and snapshot lifetimes.
This is reference sharing in CPU memory, not GPU zero-copy capture.

### Measurements

Windows NT 10.0.26300, AMD Ryzen 7 5700X3D (8 physical cores), Rust 1.99.0
(`b940084d7`), ordinary Cargo release profile, no custom CPU flags. CPU identity read from the
Windows processor registry value. No GPU or actual display is used in this benchmark.

`cargo run --release -p clipcraft-record --example performance` runs deterministic patterned BGRA
frames, 3 warm-up invocations, then five timed batches. The table reports median per-invocation
batch times. Conversion batches contain 30 frames; snapshot batches contain 10 snapshots. The
snapshot buffer contains 3,600 samples of 41,666 bytes (~150 MB), GOP 120, modeling 60 seconds at
60 fps and ~20 Mbit/s. Long-GOP insertion batches contain five invocations, each inserting 14,400
empty-payload samples with GOP 3,600 and a 3,600-frame window. Empty payloads isolate metadata cost.
Setup/payload allocation for conversion and snapshot is outside timing; insertion includes its
own buffer and sample allocations. Snapshot timing includes dropping the result.

| CPU workload | Before (ms) | After (ms) |
| --- | ---: | ---: |
| RGBA 1080p → 1080p, 1 thread | 2.352 | 1.608 |
| RGBA 1080p → 1080p, 4 threads | 1.392 | 0.638 |
| I420 1080p → 1080p, 1 thread | 4.536 | 4.392 |
| I420 1080p → 1080p, 4 threads | 1.412 | 1.416 |
| Video snapshot, 60 seconds at ~20 Mbit/s | 68.070 | 0.034 |
| Insert 14,400 samples, GOP 3,600 | 18.446 | 0.671 |

The insertion baseline was taken after introducing shared payloads, before caching GOP boundaries;
the other baselines preceded both optimizations. Runs were sequential on the same machine, before
starting the workspace build. The I420 variation is noise, not an improvement claim. Scaled
conversion results varied between runs; the example reports them, but no scaling speedup is claimed.
These short CPU measurements do not measure sustained 60 fps recording, game impact, save-to-disk
completion time, working set, or tail latency. Re-run on quiet reference machines before setting
release budgets. No two-hour soak or real Windows capture integration was performed for this change.

### Verification

The recording unit tests verify shared allocation identity, retained snapshots after eviction and
restart, cached boundary/accounting consistency across repeated eviction/clear cycles, extreme
timestamps, and pixel-buffer reuse across full frames, resizing and letterboxing. Existing
single/multi-thread pixel comparisons, hostile frame tests and fake-provider recording/hardware
fallback/audio/save integration tests remain required. See `docs/testing.md` for the canonical gate.

The release recording suite passed: 33 unit tests, 8 audio integration tests, 5 fake-hardware tests
and 13 recorder integration tests; one existing timing-only test is ignored. This is generated-media
evidence, not a run of a real Windows capture provider. The environment's sandbox temporary folder
blocked atomic writes, so tests used a writable workspace temporary directory. The default sccache
wrapper also required disabling for this environment, and Windows device tests required running
outside the sandbox. The existing recovery test that cuts files at 35/55/80% passed alone but failed
under parallel load because two cuts could recover the same fragment duration. The complete gate
passed with `RUST_TEST_THREADS=1`: formatting, all workspace tests, strict Clippy, layers, asset
attribution, licenses, bans and sources. No test assertions were changed or excluded. The CI-pinned
`cargo-deny` 0.20.2 was missing locally and was installed under the ignored
`target/performance-tools` directory; its license, ban and source checks passed. Temporary test
files live under `target/performance-test-temp`. The combined gate log is in
`target/performance-ci.log` (ignored).

After rebasing onto `d189e20`, the new compressed-export fixtures were updated to construct shared
sample payloads with `.into()`. The combined branch passed `cargo xtask ci` with serial tests and
`cargo xtask docs-check origin/main`. This includes upstream's compression and native-surface
fallback tests; it adds no real-GPU evidence. The combined gate log is
`target/performance-upstream-ci.log` (ignored).

## Remaining work, ordered by likely user impact

Windows SDR work has since implemented the compatible WGC/D3D11/Media Foundation handoff.
[Generated-window measurements](windows-performance.md) confirm zero CPU readbacks, but both CPU
and native routes still miss requested 1080p60. The table below describes the original review;
the linked report records current implementation, evidence and remaining gates.

| Priority | Area / source | Observed cost and next measurement |
| --- | --- | --- |
| 1 | Platform/capture/record: `windows.rs`, `hardware.rs`, `mf_encoder.rs`, `recorder.rs` | Windows reads a GPU texture to CPU memory, then clones a full `CaptureFrame` in `capture()`. Hardware encoding consumes CPU RGBA; Media Foundation additionally converts to NV12 and copies into an input buffer. Measure bytes copied, capture callback time and game CPU/GPU frame times at native 1080p60, 1440p60 and 4K60. Design a safe GPU-surface capture-to-encoder interface with bounded lifetimes and tested CPU fallback. This is the largest architectural candidate, not a measured ranking. |
| 2 | Capture/HDR: `clipcraft-capture/src/hdr.rs`, Windows handler | HDR detection scans half floats and tone mapping evaluates nonlinear functions per channel per pixel, allocating each output frame. Measure SDR/HDR separately; evaluate exact lookup tables or a GPU conversion path with color/error tests before changing it. |
| 3 | Capture ownership across Windows/X11/Wayland | `CaptureFrame` owns a `Vec`; repeated/stored frames clone pixel payloads. Windows also creates depadding scratch for each callback. Measure allocations and clone time; evaluate reusable/shared frame storage. Preserve source resolution changes and screenshots. |
| 4 | Conversion/record scheduling: `convert.rs`, `h264_stream.rs` | Scaled frames still build taps and scratch each frame. Conversion creates scoped OS threads on every invocation; MF conversion creates another set. Measure CPU time and p95/p99 latency, then evaluate cached conversion plans, pooled scratch and persistent bounded workers. Avoid increasing contention with the game. |
| 5 | Desktop library: `main.rs`, `organize.rs`, `thumbs.rs` | The grid visits every entry; visibility only defers thumbnail loading. Filtering/sorting runs for both counts and cards; name comparison repeatedly allocates lowercase strings. Measure frame times with 1k/10k/50k generated entries, then virtualize rows and cache sort/search results on model changes. Require screenshots and keyboard checks. |
| 6 | Engine/index: `index.rs`, library scan/probe workers | `merge_entries` searches the previous list per scanned entry and the merged list per missing entry, yielding quadratic path matching. Metadata updates clone/rewrite the index. Measure scan/refresh and repeated probe updates at 1k/10k/50k entries; use bounded path maps and batch persistence while preserving metadata and atomic recovery. |
| 7 | Save/export: `mux.rs`, `clipcraft-export` | MP4 saves intentionally write a crash-recoverable copy and the final file. Trim remuxing already avoids re-encoding, but builds/sorts an interleaving vector and allocates packet reads. Measure wall time, allocation peak and disk bytes before considering stream merging/reused read buffers. Preserve recovery, exact cuts and cancellation. |
| 8 | Player/audio/output: `clipcraft-player`, `clipcraft-audio-out` | Playback decodes full-size RGBA; frame events use an unbounded channel. Audio callback delivery already uses `try_lock`, but capture input locks a growing `VecDeque`; mixing and WSOLA allocate intermediate vectors. Measure minimized/stalled UI queue growth, 4K decode-to-upload cost, callback overruns and speed-change CPU. Evaluate a bounded latest-frame mailbox, scaled preview and preallocated audio storage. |
| 9 | Frame/time/media/CLI/xtask | Queues already cap items/bytes; edit time stays integer/rational; probes use bounded headers and file-backed reads; CLI calls engine commands. Preserve these bounds. No measured hot-path change is justified here. Profile malformed-file scan cost and verify release builds in packaging. Compiler tuning/LTO needs startup, binary size and runtime measurements rather than blanket settings. |

## Reference-hardware evidence still required

Record at least NVIDIA, AMD and Intel encoder paths, actual microphone/system sound, changing
resolution, device loss, SDR and HDR. For each run record OS, CPU/GPU/driver, storage, resolution,
fps, bitrate, codec, replay duration and backend actually selected. Compare the same game replay
without recording and with ClipCraft: median/p95/p99 CPU and GPU frame time, 1% lows, recorder CPU,
GPU encode/copy usage, working set, allocation rate, drop count, A/V offset and hotkey-to-save
latency. Include idle/minimized operation, saving while recording, disk pressure and a two-hour
soak. Repeat paired runs; report spread. Comparison with other software requires identical visual
quality, bitrate and capture settings. No universal "fastest" claim follows from these microbenchmarks.


## M11.1e — Editor export frame work and desktop responsiveness (2026-10-23)

Baseline: `3b6642a`, plus the measurement example added in this change, built before altering the
renderer. No codec, bitrate, output quality, time math or dependency changes. Source-size RGBA
pictures previously went through thumbnail filtering, a second full-size allocation and an overlay
onto a freshly allocated black canvas. The matching opaque path now returns the owned RGBA picture
as its canvas. Smaller/transparent pictures retain black-background composition, and actual scaling
retains the existing thumbnail filter. AAC channel mixing uses two bounded stack arrays. Decoder,
codec, RGBA conversion and text caches still allocate; this is not GPU zero-copy export.

### Machine and method

Linux x86_64 cloud VM, AMD EPYC 9V74, five visible logical CPUs, cgroup CPU quota four cores and
32 GiB memory limit; Rust 1.99.1, ordinary Cargo release profile, no custom CPU flags. Shared-host
scheduling and other desktop sessions introduce noise. No GPU or physical capture/audio device was
used. These measurements are not a reference-PC performance claim.

The new Rust example generates a deterministic two-second 1920×1080/60 H.264 source with a moving
luma pattern and fixed chroma. Source generation is outside export timing. Every export uses the same
software Speed preset, two encoding threads, standard 1080p/60 output plan, one text layer and
stereo silent AAC. It runs one warm-up and three measured exports. The six-second case repeats the
two-second source as three cuts. Exclusive output-directory creation prevents deleting/overwriting
old results; duration is bounded to even values from two to ten seconds. Four exports and the
fixture are retained for inspection. No display or external codec tool is required.

```sh
cargo run --release -p clipcraft-export --example sequence_performance -- target/export-perf-new 2
cargo run --release -p clipcraft-export --example sequence_performance -- target/export-perf-long 6
cargo test --release -p clipcraft-export --lib measure_sequence_composition -- --ignored --nocapture
```

Use new directories on repeat runs. To reproduce the historical full-export baseline, build the
same example against `3b6642a` (copy just the example into that checkout), then run baseline and
current binaries sequentially with identical arguments. Exclude run 0. The private baseline binary
was retained before implementation; no workspace build or native UI test ran during the final
paired two-second series or the six-second series.

### Results

| Workload | Before median (range), ms | After median (range), ms |
| --- | ---: | ---: |
| Matching 1080p RGBA composition | 43.507 (43.045–45.754) | 1.962 (0.986–2.725) |
| Matching 4K RGBA composition | 127.041 (126.373–140.749) | 5.664 (5.333–6.327) |
| 1080p → 720p composition | 18.830 (17.223–18.908) | 15.730 (14.730–16.697) |
| Portrait into landscape composition | 11.829 (11.784–14.091) | 11.700 (11.587–12.255) |
| Full two-second 1080p60 export | 12,127.885 (11,976.696–12,240.160) | 7,504.321 (7,391.795–8,237.460) |
| Full six-second 1080p60 export | Not measured | 20,722.060 (20,597.945–20,849.419) |

Composition compares the retained pre-change algorithm with the current helper in the same
release test process, alternating their order. Two warm-up batches precede five timed batches of
four pictures. Each invocation includes cloning the input RGBA picture and dropping its result;
source generation, decode, text, effects, fades, encode and mux are outside this microbenchmark.
Only the full-export rows include those stages. The scaled/portrait differences are not evidence
of a general resizing speedup. The full paired workload used about 38% less wall time; it still
runs slower than real time and cannot establish capture/game impact or real-GPU throughput.

The matching path removes two RGBA allocations per frame: 15.82 MiB of transient buffers at
1080p and 63.28 MiB at 4K, excluding decoder/codec/conversion state. An ownership regression checks
that the returned matching canvas retains the input allocation. This is an allocation-path result,
not a measured reduction in total process peak. Linux `VmHWM` at the final measured export was
570,704 KiB before and 559,392 KiB after; the six-second process reached 575,100 KiB. These are
process high-water marks including generation, warm-up and allocator retention; they are not
export-only memory, leak/soak evidence or a universal memory cap. Longer sources/GOPs and physical
GPU adapters require separate measurements.

All two-second benchmark inputs and outputs compared across baseline/current runs were
byte-identical: the export had 120 pictures and 4,379,816 bytes (SHA-256
`278871f354531316712f8de7430ae53d0b6eb05c6275e4116914369c2c1b470e`). The six-second outputs each
have 360 pictures and 13,483,974 bytes. Output/source inspection uses generated media only.

### Responsiveness and limits

The existing single background export worker remains. Latest progress is stored for every callback,
but progress-driven event-loop wakeups are spaced at least 100 ms apart; completion/errors wake
immediately. Preview and input can redraw independently. Space and arrow-key transport now remain
available during export, matching the mouse controls. Edits and a second export remain blocked,
including while cancellation is pending. Cancel displays its pending state until the worker returns;
progress/cancellation live in the top toolbar so a short window cannot clip them below the timeline.
Disconnected workers surface an error. Cancellation is checked after decode and composition before
starting encode, but cannot interrupt a codec or filesystem call already executing. No new worker
pool, frame queue, priority scheduler, throughput promise or public engine command was added.
Development builds now optimize `clipcraft-export` at level 3, matching the existing player/codec
policy, so first-party pixel loops do not run unoptimized. This does not affect the release-profile
before/after numbers above.

Pixel regressions compare the original composition across source sizes, alpha, all colour effects
and fade amounts, including reuse of the matching RGBA allocation. Existing generated export tests
cover shared text rendering, audio/cut timestamps, 30/60 FPS, size targets, hardware stand-ins,
cancellation, collision safety and unchanged sources. Native workflow and final gate evidence is
recorded in [testing.md](testing.md); physical GPU/Windows/macOS measurements remain open.
