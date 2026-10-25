# Current performance opportunities — 2026-10-25

Audited fetched main **`f0ed9c2`**, after M11.1i GPU composition, M11.1j cleanup and M5.2b Windows
SDR GPU scaling. This is a proposed optimization order, not implemented speedups or a promise of
physical-device performance. The review covers all 13 workspace packages; measurements below
exercise only the stated generated workloads. No product code was changed for this audit.

**Implementation follow-up:** M2.P2 implements priorities 1–3; see the current measurements and
remaining limits in [performance.md](performance.md) and [ADR 0027](adr/0027-responsive-library.md).
The table below remains the audit's proposed order at its stated baseline, not a list of still-open work.
The editor browser already had virtual rows at that baseline; its remaining work was cached search,
whereas the main clip grid required virtualization.

## Measured findings

### Idle keyboard focus drives continuous redraw

`apps/clipcraft/src/main.rs::draw_focus_ring` calls `context.request_repaint()` whenever keyboard
focus is visible. That ring is static: after Tab, it schedules another frame, which schedules
another, until a pointer press clears `keyboard_focus`. Each redraw also rebuilds the theme,
synchronizes presence and runs the current screen's UI work. This is the first fix to make.

The existing development binary (matching this revision's Rust source; debug symbols disabled,
normal package optimization policy) ran on the managed Debian 13 cloud CPU with a private
1440×900 Xvfb/GL software-rendered desktop. Isolated generated library, capture stopped, Discord
disabled. After startup settled, three sequential 3-second `/proc/<pid>/stat` CPU observations:

| Condition | Process CPU, 100% = one logical CPU |
| --- | ---: |
| Idle after mouse click | 5.67% |
| Idle after two Tab presses, focus ring visible | 347.58% |
| Idle after another mouse click | 5.00% |

These are short observations on a software-rendered **development build**, not release/gaming-PC
CPU budgets. The before/after condition reverses the effect in the same process; source tracing
identifies the self-sustaining repaint request. No GPU utilization, power or frame-time claim.
Evidence: `/tmp/clipcraft-idle-audit.json`, ignored native screenshot
`.clipcraft-local/playback-priority-test/optimization-audit-tab-focus.png`. Existing scans/timers
still produce periodic work without the focus loop; fixing it does not imply zero idle CPU.

### Library view repeats expensive sorting

`library_content` calls `filtered_entries` to obtain a count and again for cards. That helper sorts
before applying the selected category. `compare_media_names` allocates two lowercase strings per
comparison, also used to break date-sort ties. `clip_grid` builds every card regardless of visibility;
only thumbnail loading is visibility-gated. The editor Library already virtualizes rows but rebuilds its search result list each UI frame. Cache and virtualize these before adding more rendering infrastructure.

A standalone safe-Rust diagnostic extracted the current `visible_entries` and comparator **unchanged**
from `main.rs`; a small entry struct supplies exactly the fields they read. Generated shuffled clip
names (including Unicode), paths and dates, All clips/empty query. Compared two current sorts
(count + cards) with one candidate `sort_by_cached_key` sort, with lowercase/path keys created once
per entry. Output path order is asserted identical for both sorting modes. One warm-up, seven
alternating measured samples, medians shown; source generation is outside timing. Rust 1.99.1,
`rustc -O`, shared cloud CPU. Neither candidate nor cache was integrated into ClipCraft.

| Entries | Sort | Current two sorts | Candidate one cached-key sort |
| ---: | --- | ---: | ---: |
| 1,000 | Name | 5.077 ms | 0.200 ms |
| 1,000 | Newest | 1.416 ms | 0.202 ms |
| 5,000 | Name | 31.929 ms | 1.228 ms |
| 5,000 | Newest | 17.562 ms | 1.324 ms |
| 10,000 | Name | 74.796 ms | 2.577 ms |
| 10,000 | Newest | 45.795 ms | 2.913 ms |

5,000 is the current scan limit; 10,000 covers the maximum remembered/offline entries per folder.
These are helper timings, **not full UI frame times**: no drawing, disk reads, game filters, game
label derivation, JPEG decoding or texture uploads. Data distributions and host load matter.
Revision caching could avoid repeated sorts altogether, but its runtime benefit is unmeasured.
Local diagnostic source/log: `/tmp/clipcraft-library-audit.rs`, `/tmp/clipcraft-library-audit.log`.
No new benchmark executable is required or shipped by the product.

## Ranked implementation candidates

| Order | Area and current evidence | Proposed change | Verification / main risk |
| ---: | --- | --- | --- |
| 1 | Idle UI: `main.rs::draw_focus_ring` schedules its own next frame; measured above. `update` also calls `kit::apply_theme` on every frame | Let input/state changes repaint the static ring; apply the theme at initialization or actual theme change. Preserve genuine playback/recording/worker/timer wakes | Native Tab/mouse/idle/minimized checks and repaint-count regression; visible focus/accessibility must remain correct. Low risk, small slice |
| 2 | Library: repeated filter/sort and full card construction in `main.rs`, game-name normalization in `organize.rs`, editor Library search recomputation | Revision-keyed view indices/counts and normalized search/sort/game keys; filter category before sort. Virtualize card rows with bounded overscan | 1k/5k/10k generated libraries, count/sort/filter parity, metadata invalidation, keyboard/context-menu/scroll checks; variable card height and focus need care. Medium scope |
| 3 | Disk on UI: `library_scan.rs::poll_library_scan` calls `StoreLibraryScan`; `main.rs::save_probes`/`edit_library` execute index writes synchronously. `index.rs::write_index` serializes, writes, syncs and renames | One ordered bounded persistence worker, batched probe updates and explicit completion/errors. Keep one owner for merges and metadata edits. Map probe paths to indices rather than repeated `.find` in UI and `store_probes` | Delayed/failed disk, concurrent favorite/scan/probe edits, shutdown flush, recovery and stale-result tests. Never lose metadata or allow worker/UI overwrites; high correctness importance |
| 4 | Preview/playback: `clipcraft-player::decode_at` requests a full source picture and converts all pixels to RGBA before editor scaling. M11.1i did not change decode | Add bounded display-size decode hints and/or native-plane reduction before RGBA; full detail for actual inspection/fullscreen as needed. Treat proxy generation as a later fallback | Compare aspect/colour/text geometry, exact seek/step and A/V sync at 1080p/4K; reduced native input may differ from current filter, so preview/export parity needs explicit review. Medium/high risk |
| 5 | Export: `sequence::render` opens every segment in validation and again during rendering; repeated cuts of the same input recreate decoder caches | Reuse the current source across same-file cuts; separately validate each cut's bounds. Cache bounded source facts/preflight by file identity; evaluate a small LRU only after measuring one-source reuse | Count source opens for repeated cuts, exact reordered/seeking/fractional-cut output, changed/missing sources, memory/cancel/fallback/source preservation. One-source reuse first, no unbounded decoder pool |
| 6 | CPU capture fallback: `convert.rs` rebuilds scaled taps and per-band scratch, spawning scoped OS threads each frame. X11 clones the full frame into `last`; Windows CPU/HDR path clones stored pixel frames | Reusable size/framing conversion plans/scratch; evaluate bounded persistent workers against single-thread conversion. Shared immutable CPU frame storage can avoid fallback-history clones | Exact pixel/letterbox/resolution-change comparisons, repeated-frame/source lifetime and screenshot tests; measure allocation rate and p95/p99 latency. Changes must not regress the already working Windows SDR surface path |
| 7 | Recording/playback audio: `SampleQueue` takes a blocking mutex and grows a VecDeque; `drain_into` can grow caller output under that lock. Mixing/planarization and WSOLA allocate blocks. `AudioPacket` Vec payloads are cloned on replay snapshots | Preallocate reusable audio buffers and shorten queue lock ownership; evaluate a bounded SPSC queue only if required by callback measurements. Share immutable AAC payloads as video already does | Callback contention/overrun counters, exact samples/pitch/clock/gap/loss tests, bounded memory and save lifetime; real devices remain separate. Do not change overload behavior or audio quality for a microbenchmark |
| 8 | Editor metadata: UI edit clones the session/history; `validate_state` walks/clones snapshots, current time/offset lookup is linear. `preview_recipe` rebuilds text layers each UI frame and playback worker clones them per decoded frame | Immutable revision-keyed recipes shared with workers, prefix offsets/binary search, and bounded shared history snapshots. Validate untrusted input at command/recovery boundaries; cache facts only for immutable validated state | Undo/redo, hostile JSON, live drag edits, prepared cuts and latest-seek revisions; retain all budgets/error behavior. Metadata optimizations first, no broad engine API rewrite |
| 9 | GPU compositor: device buffers are reused, but host `ink`/`params`/axis arrays and bind groups are recreated; unchanged text masks upload every frame; output is synchronously read back then reuploaded by UI | Reuse bounded host scratch and shape-dependent bind groups; revision-cache masks/axes. Consider GPU-resident preview only after a measured prototype across egui's current GL and compositor wgpu devices | Actual GPU/CPU pixel parity, context loss, cancellation and retained-memory accounting. No physical-GPU evidence yet; avoid assuming transfer elimination is a small change or GPU always wins |
| 10 | Thumbnails/background integration: `thumbs::texture` loads cached JPEG files on the UI (up to six per frame). Probe/thumbnail result channels are unbounded and UI drains all pending messages. Presence card/IPC targets rebuild each frame | Load/decode JPEG on bounded workers; prioritize visible/nearby clips. Bound result queues and per-frame application, coalescing wakes. Cache presence output on relevant state changes while preserving clear/reconnect/status refresh | Slow cache storage, fast scroll, stalled UI, cancelled scans and stale revisions; disabled/privacy-clear behavior and fixed memory budgets. Existing background decode admission stays authoritative |

Orders reflect observed hot paths, user frequency and safe implementation size, not measured
cross-subsystem speed rankings. Start with **1 → 2 → 3**, then source reuse/reduced preview work.
CPU fallback/audio and GPU-resident work need dedicated measurements before adopting larger designs.

## Already addressed — do not optimize stale implementations

- `index::merge_entries` already uses a path HashMap and HashSet. The quadratic merge described in
  the older audit is historical. Probe-result application in `store_probes` is still linear per result.
- Windows SDR GPU capture/scale/letterbox → Media Foundation exists; HDR compatibility is opt-in
  after M5.2b. Do not propose building that same surface path again. Broader hardware/HDR/device-loss
  evidence remains in [windows-performance.md](windows-performance.md).
- Replay video snapshots already share encoded bytes and cache GOP boundaries; AAC sharing is separate.
- Playback display results already retain one latest picture and reject stale seek completions;
  foreground decode admission already exists. Remaining thumbnail/probe channels are separate.
- Preview/export already share bounded GPU/CPU composition. M11.1j removed duplicate UI text
  textures, unused audio-policy code and unused time wrappers; do not reintroduce them as optimizations.

## Coverage and constraints

Desktop/CLI, engine/index/editor, capture, record/mux/audio, playback/time stretch, audio-out,
platform backends and export/compositor were inspected. `clipcraft-time` is now just the shared
clock reexport; `clipcraft-frame` is bounded queue infrastructure; `clipcraft-media` probes bounded
headers/file-backed sources; CLI delegates to engine; xtask is developer tooling. No speculative
rewrite of these small layers is justified. Avoid blanket unsafe/SIMD, global fast-math, lossy
quality changes or compiler/LTO tuning without a baseline. Startup/package-size measurements are
needed before changing release flags. Process observation already reuses sysinfo state/caches
Steam IDs and runs off the UI; it is a lower priority than the confirmed repaint loop.

Every implementation must preserve the Rust-only native/offline product, exact Tick time,
source-preserving/cancellable exports, bounded workers/caches/queues, CPU fallback and current
privacy rules. Use independent correctness tests plus paired controlled measurements, full
`cargo xtask ci` and documentation updates; no unstable timing thresholds in CI. Windows/macOS,
real audio, physical GPU power/throughput and actual game impact remain target-device gates.
