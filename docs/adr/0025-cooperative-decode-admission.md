# ADR 0025 — Playback-first admission for worker-side decoder stages

**Status:** accepted for M11.1g, 2026-10-25; [issue #10](https://github.com/nanox333/clipcraft/issues/10).

## Problem

M11.1f bounds unread pictures and rejects stale seek results, but playback, thumbnail generation
and timeline analysis still enter media code independently. FilmCraft's background hint controls
retained decoder state; it does not give playback priority across these callers.

## Decision

Use a process-local safe-Rust coordinator, with three active stage permits, one background permit
at most, and 64 registered waiting callers. At most 48 waiters can be background, reserving the
last 16 waiting registrations for foreground arrivals. A waiting foreground request prevents new background
admission. Existing workers acquire permits; the coordinator creates no threads and queues no job,
file path or media. Logical slots reserve no CPU cores and cap no codec-internal thread count.

Foreground: player source open/metadata, video decode+RGBA conversion, and audio decode/stretch
blocks. Background: uncached thumbnail generation, timeline source open, each timeline picture,
generic waveform chunk, MP4 waveform setup and each decoded packet. Analysis releases between
chunks, so long waveforms do not retain admission for the entire request. Audio releases before
pushing buffered output or waiting on the device. Thumbnail generation also sets FilmCraft's
background hint; timeline analysis already did. All picture/audio/timing/file contracts stay.

Wait through a condvar with 20 ms cancellation checks, always invoking cancellation predicates
outside the mutex. Video waits observe stop and current seek revision; opening observes stop,
audio observes feed close, and timeline observes its cancellation flag. The legacy thumbnail API
has no cancellation argument. Superseded video waits return to apply pending controls, rather than
failing the player. An admitted call may still race cancellation; existing publication/generation
checks reject obsolete results. No codec/file call already executing is preempted.

Registration/permits use RAII; early returns and caught decoder panics release capacity. A full
waiting budget or poisoned coordinator produces existing caller failure states; no counter wrap
or silent success. Poisoned state remains unavailable until process restart. Cancellation remains
observable even when all slots are occupied. A 20 ms poll interval is not a wall-clock latency
promise under OS scheduling or a blocked codec.

## Limits and validation

No new blocking admission on UI JPEG-cache loading or metadata-only `source_info`. Recording and
export do not participate. Prepared preview players currently have the same foreground priority
as other Player instances. Caller/job/control/result queues and decoder caches retain their own
contracts. Background work may wait indefinitely under uninterrupted foreground saturation;
within each priority no FIFO ordering is promised. This is cooperative admission, not a shared
worker/decoder pool, OS priority, total RSS budget, GPU feature or measured performance claim.

Use isolated-coordinator priority/reserved-capacity/cancellation/panic/poison/pending-limit and
concurrent stress tests, then generated simultaneous playback+sound/thumbnail/waveform integration,
existing audio/seek/editor regressions and full workspace/docs gates. Hardware latency, stuttering,
CPU use and game contention need separate reference-machine measurements. No upstream Concat code,
shader, artwork, FFmpeg binding, dependency or first-party unsafe is incorporated.
