# ADR 0027 — Cached library views and ordered desktop index writes

**Status:** accepted, M2.P2 priorities 1–3, 2026-10-25; [issue #14](https://github.com/nanox333/clipcraft/issues/14).

## Decision

Stop scheduling a new frame just to paint a static keyboard focus ring. Input, media workers,
transport, capture/status timers and animation retain their own wakes. Install the current fixed
theme at native initialization instead of recreating it each update.

`library_view::ViewCache` normalizes clip/game names once on model invalidation, computes stable
name/path and date/name/path orders, and caches filtered indices/counts by category/query/sort.
Sidebar favorites/collection/game aggregates share this model cache. Mutations, scan application,
relink and captured-label refresh invalidate it; probes update facts without changing name order.
UI cards read current entries by index rather than cloning entries or keeping a second library.
The existing 5,000 scan/10,000 remembered-entry limits still apply. The editor browser already had
virtual rows; `BrowserSearch` now caches its insertion-order search indices by source Arc identity
and normalized query. A Weak identity avoids retaining replaced media pools and detects make_mut.

Main clip cards use fixed-height virtual rows and stable path-derived IDs. Keyboard navigation
moves among four controls per card, scrolls target rows into view and resets egui's automatic focus
direction when it has handled a key, avoiding double steps. Tab/Shift+Tab can cross virtual rows;
Home/End, Up/Down and PageUp/PageDown move within the grid. Text fields/popups retain normal keys.
Pointer activation establishes focus even on an offline card. Tests enforce fewer than 40 built
cards for the representative 10,000-entry/1200×700 viewport; this is not a universal widget cap.

## One desktop writer

`library_store` owns a single serial worker for existing `library.store_scan`, `library.edit`,
`library.store_probes` and `library.register_recording` engine commands. CLI commands stay synchronous.
No new user operation, command ID/schema, metadata version, dependency or first-party unsafe.
The worker calls the same engine validation, merge and atomic temp/write/sync/rename implementation.
No index serialization, sync or rename for these operations runs on the desktop UI thread.
Startup index reads, cached JPEG loads, settings writes and some explicit file metadata operations
remain separate; this is not a claim that all filesystem access is asynchronous.

At most 64 accepted jobs are outstanding, counting active work and replies until the UI consumes
them. There is a 128 MiB accounted payload budget. Scans reserve 48 MiB plus path overhead for
bounded scan/index metadata and replies; other jobs charge vector/string capacities plus overhead.
At most two scan reservations coexist, leaving room for small edits/registrations. The engine's
32 MiB index-file and per-folder bounds still apply. Active command copies/loaded engine index,
UI model/caches, one coalesced error fallback and a <=64-edit rebase journal are additional bounded
storage; this is not a total RSS cap. Admission refuses a full/stopped queue with an actionable
message before optimistic user edits replace the UI model. Probe cache batches can remain in UI
memory for a later retry. Pending scans use checked generations and late replies cannot select an
old folder. Job IDs fail closed on exhaustion.

Edits update the UI after admission; successful write replies do not replace that model. A scan
reply is merged with accepted edits newer than its submission (including global collection changes)
before replacing the view. Newer valid probe facts for unchanged files remain. The journal is
released after the relevant scan acknowledgements. Saved-game registration joins this same ordered
writer before its refresh, so a scan cannot race it through another desktop index writer.
Probe path lookup uses maps both in UI application and in the engine's persisted batch.

## Failure and shutdown

A failed/panicking command stays at the head and pauses later writes. The UI reports unsaved changes
and offers **Retry saving**. Retrying the head before newer writes prevents an old failed favorite
or collection action from overwriting a newer one. Error notifications/fallbacks are coalesced;
accepted writes are never silently evicted to make space. A raw scan may still display in memory
on persistence failure, preserving existing local metadata/captured-label fallback. Poisoned or
terminated workers fail closed; joins are checked. They may require closing/reopening rather than
recovering the queue. No worker error touches source media.

Normal close is cancelled while accepted writes are outstanding, shows saving status and closes
after acknowledgement. Failed saving offers retry or explicit **Close without saving pending
changes**. That choice can lose pending library metadata, never media; queued state is discarded
and the worker stops. A filesystem operation already in progress is not preemptible. Slow/hung
storage can keep normal closing pending; force-quit/process crash can lose unacknowledged metadata.
The last successfully published index remains the recovery boundary. There is no durability journal
or promise that an optimistic on-screen edit is saved before acknowledgement.

## Evidence

See [testing](../testing.md) and [performance](../performance.md) for failing-before/passing-after
focus regression, generated view/reference tests, keyboard/widget bounds, delayed/failing/panicking/
poisoned writer tests, queue bounds, scan/edit/probe/save ordering, shutdown and native measurements.
No physical GPU, Windows/macOS runtime, real audio or end-to-end gaming performance gate is closed.
