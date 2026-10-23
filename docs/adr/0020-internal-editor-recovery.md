# ADR 0020 — Internal editor recovery

**Status:** accepted · **Date:** 2026-10-22 · **Scope:** M11.1c

## Decision

The user approved continuing with timeline media, playback across cuts and local draft recovery.
Keep one internal recovery record in ClipCraft's local application-data directory. This supersedes
the session-lifetime-only editing limitation in ADR 0017 and ADR 0019; it does not introduce a
user-facing project format, .ccproj files, a project browser or a server dependency. Source videos
and ordinary exported MP4/GIF files remain separate from this record.

The engine owns the versioned `EditorDraft` metadata and its load/save/discard commands. UI and
CLI use the same implementation. Record current validated cuts, segment sound/effects/fades,
independent text layers, imported absolute media references, title, timeline playhead and chosen
export preset. Missing sources remain references so an unavailable drive does not invalidate the
edits; the UI probes them separately and explains that playback/export needs the source files.
No media bytes, thumbnails, decoded pictures, audio, credentials or UI textures enter recovery.

Recovery captures the current sequence without undo/redo snapshots. Restart begins a new edit
history. Validate the full supplied session, including its history, before making that snapshot;
invalid state must never become valid merely by dropping history. Preserve the existing
30-minute sequence, minimum-segment, text-layer and current-session metadata limits. A title has
at most 100 characters without control characters. Media references are deduplicated, absolute,
valid UTF-8 paths of at most 4096 bytes, with at most 5000 references including timeline sources.
Clamp the playhead to the recovered duration and validate the existing export-preset budget.

The JSON file has an ownership marker and version, with a 16 MiB serialized limit. Enforce that
limit before deserializing and while serializing. Reject malformed JSON, invalid ranges/IDs,
excess history, invalid paths/presets and unsupported versions with an actionable error. The
recovery budget can be reached before the larger in-memory sequence budget; a failed save keeps
the previous recovery and the current in-memory edit. No lossless-recovery claim is made for edits
that cannot fit the recovery budget.

## Filesystem behavior

The record uses the fixed filename `editor-draft.json` beside the existing local library index.
The API accepts an absolute path with that basename only, rejects parent traversal and symbolic
links in existing path components, and refuses non-regular targets. Custom locations are intended
for explicit CLI/test use. The default location follows the existing XDG/application-data policy;
an unavailable or unsupported location reports recovery as unavailable without preventing editing.

Save validates the new draft and any existing record, writes an exclusively created sibling
temporary, flushes it, then atomically renames it over the valid previous recovery. Unix files use
owner-only creation permissions. Failed writes/publication remove only that operation's temporary
and leave the previous record intact. Saving never writes to media paths. Atomic replacement
protects readers from partial JSON; it does not promise power-loss durability for the containing
directory or coordinate simultaneous edits in several ClipCraft processes.

An unreadable, corrupt, oversized, unrelated or future-version record is kept unchanged. Automatic
saves must not replace it silently. Explain the failure and offer explicit Discard recovery; that
operation removes only the fixed recovery file, including an invalid record the user has chosen
to discard. A missing record/discard target is a normal empty state. Do not delete sources, clear
the clip library, scan arbitrary draft directories or rename unknown files automatically.

## Interaction and validation

Load automatically on launch and identify a restored edit with a visible notice. Debounce changed
metadata and keep filesystem operations on the existing bounded worker path. Disk failures leave
the editor usable and visible; retained errors must offer an explicit retry/discard route. A
debounced save leaves a short recovery window, so recovery is a safety net, not a guarantee that
the final input before a crash is persisted. Discard recovery and a subsequent explicit edit are
separate actions; do not immediately re-save the discarded snapshot.

Require round-trip cuts/text/preset/playhead checks, fresh history after restore, missing-source
references, malformed IDs/ranges/history, metadata bounds, truncated/future JSON preservation,
oversized-save preservation, exclusive temporary cleanup, arbitrary-file and symbolic-link
rejection, repeated/empty discard, command/CLI validation and UI restore/discard checks. Record
actual test and screenshot evidence in [testing.md](../testing.md); platform capture hardware is
unaffected and remains separately verified.

M11.1d adds optional validated export quality to this record, defaulting missing fields to
720p/30 FPS. See [ADR 0021](0021-editor-export-quality.md); the ownership/version and atomic-write
protections are unchanged.
