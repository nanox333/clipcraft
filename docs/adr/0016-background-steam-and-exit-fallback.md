# ADR 0016 — Background game discovery and opt-in desktop continuation

**Status:** accepted · **Date:** 2026-10-22 · **Scope:** M7.1b

The user requested background detection, local Steam-library discovery, deterministic full-flow
verification, and an option to continue desktop recording after a game closes or crashes.

Use one bounded desktop discovery worker for process refresh, matching and local Steam metadata.
Replace pending work and discard obsolete settings generations; polling or refresh errors must
not be treated as game exits. The running-app picker uses the same worker. Steam discovery reads
bounded local `libraryfolders.vdf` and `appmanifest_*.acf` metadata, never a remote database or game
assets. A shared engine command exposes discovery to the CLI. Custom rules remain first; built-in
and Steam exclusions and the automatic-recognition switch remain effective.

Games-only remains the default. Temporary desktop continuation requires a successfully started
game session followed by its process disappearing and runs for a configurable 15–3,600 seconds
(default 60). A reopened game cancels the timer and takes capture; each subsequent exit starts a
fresh timer. A separate explicit **Desktop and games** policy maintains desktop replay even before
the first game, including at startup, and switches to game windows and back. A capture-open failure,
permission denial, unknown game, manual pause, settings change or scanner failure never authorizes
temporary desktop fallback. In either desktop mode, save the final replay before automatic source
changes/expiry. One job may be queued behind one running save, with distinct output paths; a full
queue halts frame production and retains the buffer. If the pending game exits before a save slot
opens, preserve that buffer before resolving the latest destination or stopping.
Timer expiry halts frame production even while a save is busy.
Desktop replay uses a fresh buffer and no old game label. Show desktop capture explicitly, return
to a recognized game when it starts, and stop continuation when its policy or automatic-switching
mode is turned off. Respect an explicit Stop command and existing clip/storage limits. Wayland consent and
unsupported platform/window restrictions remain; this adds no capture bypass or platform promise.

Implementation must test bounded parser failures, precedence/exclusions, worker replacement and
failure, default-off/migration, launch/save/exit/crash/pause/re-entry transitions and original-file
preservation with synthetic sources. Native UI screenshots and keyboard checks are required.
