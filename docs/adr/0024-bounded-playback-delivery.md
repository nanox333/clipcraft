# ADR 0024 — Latest-picture playback delivery and seek revisions

**Status:** accepted for user-requested M11.1f, 2026-10-25; [issue #9](https://github.com/nanox333/clipcraft/issues/9).

## Problem

At `ba03a60`, playback decodes on a worker but sends every RGBA frame into an unbounded event
channel. A stalled UI accumulates pictures it will immediately replace. A decoder call already
in flight can publish the old target after a newer seek is requested.

## Decision

Use a safe-Rust mutex-protected mailbox with three optional slots: opening metadata, latest frame,
and terminal status. A producer replaces the old frame without waiting for consumer capacity.
Polling takes at most three events in opening/frame/terminal order. Final frame precedes Ended;
fatal failure closes publication and remains available for polling. Notifications run outside locks.
No UI method, engine command ID, source media, export pixels or integer clock contract changes.

Seek, step and play/replay requests increment a checked revision and clear pending pixels/old Ended.
Serialize revision/invalidation and control enqueueing under that same mutex, including concurrent
callers. The worker carries each command's revision; frame/Ended publication checks it under the
mutex. Superseded results are skipped without ending the worker. Opening/fatal failures are not
hidden by a seek. Apply commands queued during opening before decoding its first frame.

Shutdown clears pending display slots and enqueues stop. Queue-capacity waits are never introduced;
control enqueueing uses the existing unbounded channel. Poisoned mailbox state reports Crashed and
requests worker shutdown; revision exhaustion fails closed instead of wrapping into a valid old
revision. Decoder/file calls remain non-interruptible. Worker callbacks never hold the mailbox lock.

Validate dimensions before RGBA conversion: nonzero, at most 8192 per edge, and 33,177,600 pixels
(8K UHD pixel budget, portrait allowed), with checked usize byte arithmetic. Reject oversized
metadata before decoding where available; validate returned-frame dimensions again. Decoder scratch
and malicious codec allocations before returning a frame are outside this result-buffer bound.

## Consequences and evidence

At most one pending RGBA picture is retained. Displayed/polled frames, in-flight conversion, codec
GOP/reference caches and lifecycle metadata are separate allocations; this is not a total RSS cap.
Existing command/audio channels remain separate and may still queue controls. A shared bounded
priority scheduler, decoder pooling, preview proxies and GPU compositing require later issues.

Use deterministic burst, publication-race, concurrent enqueue-order, lifecycle/failure/stop/poison/
overflow and malformed-dimension tests, plus a generated MP4 deliberately unpolled through end.
Existing audio, stepping, looping, editor/prepared-player tests and the full gate remain required.
No actual GPU or speed claim follows from this change.

The preceding review of Concat at `971ad1d` informed product priorities only. No Concat code,
shader, artwork, FFmpeg binding or dependency is incorporated. This is independent code using the
existing FilmCraft source/clock interfaces and standard Rust synchronization, with no new unsafe.
