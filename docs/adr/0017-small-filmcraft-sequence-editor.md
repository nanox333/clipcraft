# ADR 0017 — Bounded FilmCraft sequence editing

**Status:** accepted · **Date:** 2026-10-22 · **Scope:** M11.1a

The user explicitly requested the small editor expansion now, FilmCraft reuse, and a reference
layout with media at the left, a large preview and a timeline underneath. This authorizes the
portable cuts/arrangement slice ahead of the broad M8/M10 dependencies in M11.1. It does not
establish platform parity or authorize a full editor, project format, effects or plugin runtime.

Consume `filmcraft-edit` and `filmcraft-project` from the existing pinned FilmCraft revision
`6c3948a36c80e4996d924766ba7157eec4fca85e`, under MIT OR Apache-2.0. `filmcraft-project` was
already resolved transitively; the direct dependency supplies transient Track/TrackItem types.
`filmcraft-edit::split_track_at` performs the razor operation. Its general speed-aware source
mapping uses floating point, so this fixed-speed slice retains its exact integer source boundary
when translating the result. Bounds and ID headroom are checked before invoking it. No FilmCraft
project-file or whole application workflow is adopted, and no unsafe platform boundary changes.
This supersedes ADR 0004's exclusion of these two crates for this bounded use only.

A dedicated Editor rail destination owns its media library. Send to editor imports references
from Clips; Import clips/file drops probe media on a bounded worker. Right-click Details/Add to
timeline replaces per-thumbnail Add buttons. Importing does not change the main library root.

The engine exposes `editor.import`, `editor.edit` over an immutable input/copy result and `editor.export` for a
validated ordered range list. A session has one gapless video track with linked source audio,
30 minutes, a 1/30-second minimum segment and at most 64 combined undo/redo snapshots within
an 8 MiB history budget; current sequence metadata is bounded to 64 MiB. There is no separate
user-facing segment-count cap. Duration/minimum-frame budgets bound worst-case work.
Edits are in memory and remain when navigating away; exiting discards them. There is no .ccproj
or persistent draft. CLI JSON is a transient command payload, not a versioned document format.
Append/trim/reorder/duplicate/delete/volume/reset and undo/redo validate before publishing state.

Reuse FilmCraft media decoding, H.264 encoding, AAC encoding, MP4 muxing and RGBA conversion.
Sequence export renders frames sequentially at 30 fps on a 16:9 canvas, first-source height up to
720p. Fit images with black bars and no upscaling. Normalize the first audio track's first two
channels to 48 kHz stereo (mono duplicated), with per-segment attenuation/mute and silence for
missing audio. Use one continuous AAC encoder across all cuts and one initial priming edit;
concatenating independently primed AAC ranges or multiple MP4 edits is not used. Timeline time
is integer Tick throughout; video samples the timeline on its 30 fps grid, audio on 48 kHz.

One export worker and one source/cache at a time bound background work. Retain at most one decoded
picture/audio block at a time; decoder cache is 64 MiB, input header/sample and picture limits use
the existing bounded adapters. Catch codec panics as fallible export errors. Create temporary
output exclusively and publish via no-replace hard link, preserving inputs and existing output
on failure, collision or cancellation. Output filesystems must support hard links.

Preview reuses the existing player/audio output, with a source switched at each cut. It can pause
while the next source opens, and no real-time 4K performance guarantee is made. Text, transitions,
effects, overlays, detached audio, additional audio streams, HDR, speed changes and arbitrary
export settings remain deferred. Quick trim and its original-quality path remain available.

Validation requires generated multi-source visual/audio checks, exact edit/history and malformed
state tests, cancellation/publication collisions, native UI screenshots/keyboard checks, the
canonical Linux gate and documentation gate. Windows/macOS hardware and reference-machine timing
remain separate checks. See [editor.md](../editor.md) and [testing.md](../testing.md).
