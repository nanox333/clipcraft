# Performance review

**Current source audit:** [2026-10-25 review at `f0ed9c2`](performance-audit-2026-10-25.md),
including a measured idle-focus redraw loop and library-sort diagnostic. Start there for the
current optimization order; the earlier capture/merge descriptions below are historical.

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
[Generated-window measurements](windows-performance.md) confirm zero CPU readbacks and a passing
one-minute native SDR 1080p60 throughput check. Default HDR conversion, real-game impact, a two-hour
60 fps soak and other GPUs remain open; the earlier 32 fps wait stall remains unexplained.
The table below describes the original review;
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
| 8 | Player/audio/output: `clipcraft-player`, `clipcraft-audio-out` | Playback decodes full-size RGBA; M11.1f now retains only the newest pending display picture and discards stale seek results. The original unbounded event channel is removed. Audio callback delivery already uses `try_lock`, but capture input locks a growing `VecDeque`; mixing and WSOLA allocate intermediate vectors. Measure minimized/stalled UI queue growth, 4K decode-to-upload cost, callback overruns and speed-change CPU. M11.1g now coordinates playback-first decode admission; evaluate pooled decoder reuse, scaled preview and preallocated audio storage. Decoder caches/control/audio channels remain separate. |
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

## M11.1f — Bound unread playback results (2026-10-25)

Baseline `ba03a60` sends every decoded RGBA picture through an unbounded event channel. The new
mailbox holds only the latest picture plus fixed opening/terminal slots, so a UI stall cannot
accumulate historical display frames. A synthetic 10,000-publication test uses 4×4 RGBA pictures:
the retained pixel payload is 64 bytes, and the eventual poll returns opening/latest/end only.
At 1920×1080, one pending RGBA picture is 8,294,400 bytes (7.91 MiB); at 3840×2160 it is
33,177,600 bytes (31.64 MiB). These are exact payload counts, not process RSS measurements.
An in-flight decode, a returned/displayed UI picture, codec references and allocator retention
can all coexist with that slot. The pre-conversion native picture limit is 33,177,600 pixels,
8192 per edge, hence at most 132,710,400 RGBA bytes (126.56 MiB) per pending picture.

Seek/step/replay invalidate pending pixels/end markers and identify in-flight results by a
serialized revision. Superseded publication skips its UI wake; valid new results continue. Commands
already queued during source opening are applied before the first picture decode. Existing integer
clock/frame skipping, audio feed and prepared-source behavior remain. No decoder interruption,
background-work priority scheduler, control/audio-channel bound or export acceleration is claimed.

Concat `971ad1d9214e85755c05edc8777c05b4d4e255dc` informed the prior technique assessment only.
This is an independent Rust implementation; no AGPL code/shader, FFmpeg dependency or upstream
asset was imported. CPU/GPU latency, power, real-game contention and before/after total memory
still require reference-machine measurements. The deterministic byte/order tests and generated
MP4 stalled-consumer regression establish the result-retention contract without a speed claim.

## M11.1g — Coordinate worker-side decode admission (2026-10-25)

Baseline `f7004b1` limits unread display frames but has no shared decoder admission priority.
The new coordinator allows three active stages and one background stage at most; up to 64 callers
can wait, with at most 48 background waiters so they cannot fill the final 16 registrations.
New background work waits behind registered foreground requests. Metadata is fixed
counters/condvar/mutex, not a media/job queue, and it creates no threads. These are explicit work
admission bounds; codec internals may themselves use several threads or retain large buffers.

Player open/video and audio blocks have foreground priority; thumbnail generation and timeline
pictures/waveform chunks have background priority. Background analysis yields between pictures,
generic audio chunks and MP4 packets. The two slots unavailable to background work can admit video
and audio while one background stage runs. They are not reserved cores and cannot promise either
will finish within a frame budget. Existing codec/file calls are not preempted. Foreground
saturation can delay background jobs; there is no FIFO/background progress guarantee in that case.

Portable tests prove ordering, cancellation registration cleanup, panic/poison/capacity behavior,
and concurrency limits. Generated playback/PCM sound, three concurrent thumbnail/waveform callers
and source-byte checks exercise integration. No before/after playback latency, jitter, CPU, power,
RSS, export acceleration or game interference measurement was run. Existing worker/result/control
queues remain separate; decoder pooling and preview proxies are follow-up work. M11.1i below
adds optional shared GPU composition.
No Concat source/shader, native sidecar, new dependency or first-party unsafe was added.

## M11.1i — Shared GPU composition (2026-10-25)

`clipcraft-export::compositor` now runs the same fixed scaling/effect/fade/text contract for
editor preview and sequence export. Hardware is optional: automatic selection rejects software
adapters, probes hardware against CPU and uses CPU for missing/busy/failed contexts. Matching
opaque frames without edits return their owned pixels directly, avoiding GPU transfer entirely.

A strict explicit diagnostic exercised **llvmpipe (LLVM 19.1.7, 256 bits), Vulkan**, on the managed
Debian 13 CPU container. It passed 96 cases within one 8-bit channel level: four source/canvas
pairs (matching, downscaled, portrait, ultrawide), eight looks including None, three fade variants,
alpha and two ordered overlapping/clipped FilmCraft text masks. This is actual shader execution,
not mocked GPU success or automatic CPU fallback. An actual destroyed software context also
retired and completed the reference frame on CPU. Physical-GPU/device-loss evidence remains open.

Transfer-inclusive stage measurements from `cargo run -p clipcraft-export --example compositor_verify`:

| Canvas | CPU median | Software Vulkan median |
| --- | ---: | ---: |
| 1280×720 | 16.379 ms | 13.274 ms |
| 1920×1080 | 37.104 ms | 22.151 ms |
| 3840×2160 | 150.283 ms | 84.034 ms |

One warm-up followed by five measured frames per backend, median shown. Generated opaque RGBA,
matching canvas, Sepia and 37% white fade, no text in timed frames; parity cases include text.
Each GPU timing includes buffer upload, submit, poll, readback and returned RGBA allocation.
A final repeat also passed all 96 cases; CPU/software medians were 18.009/9.220 ms (720p),
34.945/21.024 ms (1080p), and 146.013/84.131 ms (4K), illustrating short-run variability
(`/tmp/clipcraft-gpu-final-verify.log`). Source clone is outside the timer. Optimized development profile (export opt-level 3, dependencies
2), Rust 1.99.1, debug symbols locally disabled to fit workspace disk. Not a release/H.264/end-to-end
benchmark, throughput guarantee or physical-GPU speedup; llvmpipe itself runs on CPU. Host load and
software-driver threading affect these short samples. Log: `/tmp/clipcraft-gpu-verify.log`.

Decode, export's existing native YUV box reduction/RGBA conversion, and text glyph rasterization
remain CPU work. Each composed GPU frame is read back before UI texture upload or encoding.
Managed reusable GPU buffers are bounded to about 206 MiB at maximum supported dimensions/masks
and retain high-water capacity until release/restart. Driver storage, host copies, text/decoder
caches and UI textures are additional. This is neither zero-copy nor a total memory cap. The
shared GPU mutex never queues a frame: contention chooses CPU. First device/shader initialization
is lazy on a worker and may block other workers initializing it; native driver calls cannot be
preempted. Poll/map waits each have a 500 ms limit. Reference GPUs, Metal/DX12, power, real-game
contention and end-to-end preview/export need separate measurements before tuning thresholds or
making hardware claims. Decoder reuse/proxies remain follow-up work.


## M11.1j — Remove redundant UI work (2026-10-25)

After M11.1i, all production editor frames already contain effects/fades/text. The old UI painter
still rasterized active text and uploaded a separate <=16 MiB text texture cache that was never
drawn in that workflow. This cleanup removes that cache and the obsolete UI paint/recolour paths.
Only the selected active layer needs UI geometry: one content key/canvas/size is retained for
handles. A bounded raster is computed on key/canvas changes then immediately discarded; unselected
layers get no UI raster. The worker and export still own their existing bounded pixel caches.
Matching-size source frame upload to egui, CPU fallback and GPU transfer costs remain. This is
structural removal of unused work, not a measured FPS/total-RSS or physical-GPU speedup claim.
Uncalled game/time wrappers and the unintegrated audio-timeline policy are also removed; active
recording/audio/timing behavior and independent pixel reference tests are unchanged.

## M2.P2 — Idle and library responsiveness (2026-10-25)

Implements the first three priorities of the [audit](performance-audit-2026-10-25.md) at `8022671`:
static focus redraw removal/theme initialization, cached views/virtual cards, and serial background
index writes. Remaining decode/audio/capture/GPU proposals stay separate. No codec/quality/time or
source media change. See [ADR 0027](adr/0027-responsive-library.md) for ownership and bounds.

### Native idle observation

Same managed Debian 13 cloud desktop, 1440×900 Xvfb/GL software rendering, development optimization
policy and isolated generated library as the audit; debug symbols locally omitted, Discord/capture
disabled. Process CPU includes its renderer threads, with 100% meaning one logical CPU. Sequential
3-second observations, not release/gaming-PC budgets or tail-latency measurements:

| Condition | Audit baseline | M2.P2 |
| --- | ---: | ---: |
| Mouse idle before Tab | 5.67% | 5.33% |
| Keyboard-focus idle after Tab | 347.58% | 5.67% |
| Mouse idle after focus cleared | 5.00% | 5.67% |

Removing the focus ring's request for its own next frame breaks the loop; legitimate input,
recording/transport/status and worker wakes remain. A regression fails with the old request
restored (`0ns` next repaint), and passes after removal. Theme setup also moves out of each update.
Short observations on a shared software-rendered development host are evidence of this idle bug,
not a universal CPU/power claim. Logs: `/tmp/clipcraft-idle-audit.json`,
`/tmp/clipcraft-idle-after.json`, `/tmp/clipcraft-focus-before-test.log`,
`/tmp/clipcraft-focus-after-test.log`.

### Actual cache helper diagnostic

The standalone Rust diagnostic includes the **actual `library_view.rs`** implementation and links
the built engine for real `MediaEntry` types. The old `visible_entries`/comparator are extracted
unchanged as the reference. Generated shuffled Unicode names/paths/dates and explicit fixture game,
All clips/empty query/name order. Result order is asserted equal. One warm-up, seven alternating
samples, median; Rust 1.99.1, `rustc -O`, same process/shared CPU. Generation is outside timing:

| Entries | Old count + cards: two sorts | New model + view rebuild | Unchanged view prepare |
| ---: | ---: | ---: | ---: |
| 1,000 | 5.353 ms | 0.309 ms | 0.000091 ms |
| 5,000 | 32.660 ms | 1.857 ms | 0.000092 ms |
| 10,000 | 77.887 ms | 3.571 ms | 0.000092 ms |

The new rebuild includes normalized names/game keys, sidebar aggregates and both sort orders;
steady prepare is averaged over 1,000 calls per measured sample. These are algorithm/helper
measurements, **not full UI frame times**: no egui drawing, GPU upload, JPEG I/O, filesystem scan,
index sync, real media or game contention. Visible-card work is verified separately: the 10,000-entry
1200×700 regression builds fewer than 40 cards and reaches both ends by keyboard. Native 5,000-entry
screenshots show scrolling, Home/End, resizing and favorite persistence. Cached JPEG loads remain
on the UI, so scrolling slow storage can still stall. The editor already had virtual rows; it now
caches insertion-order search results by Arc identity/query rather than rebuilding them each frame.
Source/log: `/tmp/clipcraft-library-cache-bench.rs`, `/tmp/clipcraft-library-cache-bench.log`.

### Persistence changes and remaining costs

One worker runs existing scan-merge, metadata-edit, probe-batch and captured-label commands in order.
The UI no longer serializes/syncs/renames the index for them. Delayed fake storage proves admission
and polling remain usable; controlled scan completion followed by newer edits/probes/registration
survives reload without reverting optimistic UI edits. Failure/panic pauses at the head for retry,
not reordering. Probe paths use maps instead of repeated linear searches in UI and persisted batches.
No end-to-end disk latency or storage-throughput improvement is claimed; writes still load/validate/
serialize the bounded index, and optimistic edit staging still clones current metadata.

64 outstanding jobs include active work/replies; 128 MiB accounted payload, 48 MiB reserved per scan,
at most two concurrent scan reservations. UI data, transient command copies/loaded index, one
coalesced failure fallback and the <=64-edit rebase journal are additional bounded storage, not
process RSS. Normal shutdown waits for acknowledgement; failed saving can be retried or explicitly
discarded. Non-preemptible storage can keep closing pending, and force quit/crash can lose pending
metadata. Media is untouched and atomic index publication remains. Startup reads, cached JPEG loads,
settings writes and some explicit file metadata checks remain separate optimization candidates.

## Software encoder pipeline (2026-10-26)

Measured on one Windows machine (16 logical cores, AMD graphics card) by recording a 1080p60 desktop
with `clipcraft-cli capture record`, 15 seconds plus the post-roll, 1,202 pictures captured:

| Encoder | Before | After |
| --- | --- | --- |
| Processor, Fast | 290 of 781 dropped (about 37 %) | 0 dropped |
| Processor, Balanced | 387 of 782 dropped (about 49 %) | 8 dropped |
| Processor, Quality | not measured | 167 dropped (about 14 %) |
| Graphics card | 15 dropped at start (counted as drops) | 0 dropped |

Changes: the software encoder runs on its own thread, so the next picture is read back and converted
while the encoder works on the last one (the two used to add up inside one picture time); the staging
texture used to read a picture back from the graphics card is kept between pictures instead of being
created for every picture, and the buffer is no longer zero-filled before it is overwritten; BGRA to
I420 conversion reads each pixel as one 32-bit word (single-thread time for 1080p fell from 4.6 ms to
2.8 ms, bit-identical output); the automatic thread count for the software encoder is up to eight
(half the cores) instead of four; pictures replaced while the encoder is still starting are no longer
counted as drops. A pure-Rust alternative encoder (`rusty_h264-encoder` 0.16.1) was measured at 4-5x
slower than the current one on the same synthetic 1080p60 content and was not adopted. Quality effort
on the processor still cannot sustain 1080p60 on this machine; the Overview suggests the graphics card.

## Graphics card, timers and playback (2026-10-26)

Measured on the same Windows machine (AMD graphics card, 2560x1440 desktop at 60 fps, H.264 unless
noted). GPU engine figures are Windows' per-process "GPU Engine" counters averaged over several
seconds; the other figures are from small harnesses around the real recorder and player.

| What | Before | After |
| --- | --- | --- |
| Graphics card video engine, effort Fast | 26 % | 14.4 % |
| Graphics card video engine, effort Balanced or Quality | 26 % | 26 % (unchanged) |
| Graphics memory used by recording (no window) | 130 MB | 84 MB |
| In-app playback of a 1440p60 clip, steady state | 30.6 fps shown, 93 % of pictures skipped | 60.0 fps shown, 1 of 536 skipped |
| Same clip as H.265 | 29.4 fps | 59.5 fps |
| Recording, processor used by the recorder alone | about 7 % of one core | not changed |

Causes and changes:

- **Encoder effort now reaches the graphics card.** The Fast, Balanced and Quality setting only
  affected the processor encoder. It now sets Media Foundation's quality-versus-speed value for the
  card (0, 75 or 100). On AMD the encoder has two tiers, split at 50, and the fast tier uses about
  half the video engine. Measured cost of the fast tier on game-like 1080p60 motion at the same 8 Mbps:
  PSNR 31.09 dB against 31.65 dB, SSIM 0.895 against 0.909. Balanced and Quality keep today's quality.
- **Windows rounds short sleeps up to 15.6 ms.** Player pacing slept 5 ms and woke after 15 ms, so
  every other picture was skipped. While recording or while a clip or the editor is open the program now
  asks for a 1 ms timer (`set_fine_timer`) and gives it back afterwards.
- **Playback decodes ahead.** About one picture in ten is a full intra picture that takes 30-80 ms to
  decode at 1440p, against about 5 ms for the rest. A queue of six finished pictures
  (`clipcraft-player/src/ahead.rs`, bounded to 96 MB) lets the slow ones be decoded while the fast ones
  are shown, and the playback clock now counts from its last deadline instead of from the moment it woke
  up, which had dropped a picture every few seconds.
- **Smaller Media Foundation picture pool.** At most four converted pictures are allocated for the
  encoder instead of eight, which is where the 46 MB came from. No frames were dropped at 60 or 120 fps.

Left alone because it measured as small: the recorder's own thread costs (about 3 % of a core for the
hardware encode thread), the idle window (about 1 % of a core, mostly game-process discovery), and startup
(the window appears in well under a second). Hardware video decoding was tried for playback and was no
faster than the software decoder on this machine, so it stays off.

## Playback processor cost (2026-10-26)

Playing a 1440p60 H.264 clip through `clipcraft-player` (a harness polling the player, no window)
used about 455 % of one core and 750 MB. The measured split: sixteen idle-then-spinning workers in
FilmCraft's thread pool, woken to convert each picture to RGBA, took about 340 %; the decoder's own
pool and the rest took the remainder.

- **Fixed-point RGBA conversion** (`clipcraft-player/src/rgba.rs`) replaces FilmCraft's floating-point
  `to_rgba8` for 8-bit Y'CbCr in BT.709 or sRGB. Table lookups, two pixels per chroma sample, a branch-free
  clamp, and at most four threads in bands: about 8 ms of processor time for a 1440p picture instead of
  tens of milliseconds spread over every core, within one level of FilmCraft's result on every layout and
  range (checked on random data). Other colour spaces still use FilmCraft's path.
- Result: about 250 % of one core and the same 60 fps (834 of 840 pictures delivered over 14 s).

Measured and not adopted:

- **Hardware decoding for playback** was no cheaper (about 450 % of one core) and delivered fewer
  pictures (611 against 836), so it stays off.
- **FilmCraft's "background" decode hint** limits the H.264 decoder to two threads: 183 % of one core
  and 557 MB, but seeks took a median 304 ms against 212 ms. Smooth scrubbing was judged worth more than
  the saved processor time, so playback keeps the full thread count.
- A random-data benchmark of the new conversion first looked 5x slower than FilmCraft's, because
  unpredictable values defeat the branch predictor; picture-like data shows the real cost.

## Does a lower GPU priority protect a game? (2026-10-26)

No, on this machine. A stand-in game (a full-screen pixel-shader pass at 2560x1440 in its own process,
kept three passes deep, so it always wants the whole card) was run for 17 seconds, with ClipCraft's
recorder (1440p60, graphics-card H.264) started two seconds in. `IDXGIDevice::SetGPUThreadPriority`
was applied to the recorder's Direct3D device (capture copies and the video processor) at -7 and +7,
each run two or three times, with 2 ms passes (about 160 per second) and 1 ms passes (about 1,000).

| Recorder GPU priority | Game with 2 ms passes | Recorder frames dropped (of 900) | Game with 1 ms passes |
| --- | --- | --- | --- |
| Game alone | 160.6 / s | | 1,008 / s |
| Normal | 151.9 / s (-5.4 %) | 54 | 957 / s (-5.1 %) |
| -7 (less than the game) | 151.8 / s (-5.5 %) | 27 | 954 / s (-5.4 %) |
| +7 (more than the game) | 154.8 / s (-3.6 %) | 195 | 954 / s (-5.4 %) |

The game's loss does not depend on the priority, so no priority setting was added. It does depend on
the recorder simply being active: about 4 % at 5 fps and about 7.5 % at 120 fps, 5 % at 30 or 60 fps,
and the same with effort Fast, a 720p picture, or H.265. It is mostly a fixed cost of capturing the
screen, not of ClipCraft's own copy and conversion work. Under a saturating game the recorder dropped
3-6 % of its frames at 2 ms passes (none with 1 ms passes); the cause is the game's long passes
delaying the recorder's, not something the priority fixes (+7 made it worse).

## Software H.264 encoder: a second measurement on 1080p (2026-10-28)

The speed work listed in `crates/filmcraft-h264enc/UPSTREAM.md` (static-content skip, zero-residual
shortcut, deblock and intra fast paths) was re-measured on 1080p recordings with
`cargo run --release -p clipcraft-record --example encoder_hash W H FILE`, which prints a hash of every
output byte, the size and the best-of-three milliseconds per picture for 8 settings. Profiling the
original FilmCraft copy had shown that motion search was not the cost: deciding that an unchanged
macroblock needs no residual took 45-65% of the time.

| 1080p, Speed preset (game UI clip unless noted) | Original FilmCraft | Now |
| --- | --- | --- |
| QP 28, 1 thread | 32.1 ms/picture | 7.3 ms |
| QP 20, 1 thread | 30.6 ms | 9.7 ms |
| Recorder setting (VBR 12 Mbit/s, adaptive quantisation, 2 threads) | 40.9 ms | 13.9 ms, size -1.6% |
| Balanced preset QP 28 | 51.8 ms | 39.7 ms |
| IDE desktop clip, QP 28, 1 thread | 35.7 ms | 20.1 ms |
| IDE desktop clip, recorder setting | 37.9 ms | 25.4 ms, size -4.2% (measured while another job ran) |

Sizes stay within 0.3% of the original at equal QP. For reference, x264 `veryfast` encodes the same
frames in 7-9 ms on one thread. Not yet done: faster Balanced/Quality presets, keyframe stalls, lazy
half-pel planes, rate-control undershoot (a 12 Mbit/s target produced 9 Mbit/s), and measuring real
high-motion gameplay.

Compression, 2026-10-28 (IDE desktop clip, 1080p, QP 28, 1 thread; harness `encoder_hash` with
`ENCODER_HASH_STREAM=<dir>` keeps the streams for an ffmpeg PSNR check). Speed made 2.00 MB at
42.5 dB, Balanced 1.29 MB, x264 `veryfast` about 0.7 MB at 43.1 dB: the Speed preset was missing
inter partitions. Adding them plus a finer motion search gives 1.32 MB at 42.4 dB (-34%) for
20 -> 31 ms per picture; the game UI clip is unchanged in size (+10% time). 8x8 intra and the
adaptive 8x8 transform did not help in Speed. The gap to x264 on desktop content is now about 1.9x.
