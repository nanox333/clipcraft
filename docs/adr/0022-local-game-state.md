# ADR 0022 — Optional local match-data adapters

**Status:** accepted for the user-requested CS2/Dota 2/League prototype, 2026-10-23.

## Context

The user asked to implement these three games after discussing richer local information than
Discord normally displays. They explicitly authorized the local GSI/API approach. Core capture
must remain offline, Rust-only and independent of any game integration or hosted service.

## Decision

Engine commands validate/normalize allowlisted game snapshots and exclusively install Valve
configs. `RecordingSettings.game_integrations` defaults all games off. One platform-layer Rust
worker owns authenticated CS2/Dota loopback listeners and a read-only League loopback HTTPS client.
The desktop and CLI use the same engine parse/install/settings operations. Platform may depend
on engine (L5 → L4); engine never depends on platform. No control endpoint is introduced.

This authorizes optional local game-protocol listeners inside the app, not a required server
process or hosted service. "No telemetry" continues to prohibit outbound product/user tracking;
opt-in local game-state information is the expressly requested exception in terminology only.

Use existing permissive `getrandom` 0.3 (MIT OR Apache-2.0) for a 256-bit OS-generated token,
stored separately from settings. Add `native-tls` 0.2 (MIT OR Apache-2.0) only on Windows/macOS,
where its Schannel/Security.framework bindings call operating-system TLS implementations. No
bundled C TLS implementation or non-Rust first-party component is introduced. Linux has no
League client transport. Do not use a TLS provider that introduces bundled C cryptography merely
to read a loopback endpoint; do not disable certificate or hostname verification.

The unmodified `riotgames.pem` is Riot's publicly documented CA certificate: public cryptographic
trust material, not game artwork or copied executable code. Its source and SHA-256 are recorded
in attribution. Only this CA is added to normal platform trust. No network fetch happens at runtime.

## Consequences and limits

The feature retains transient allowlisted fields only, with bounded input, IO, polling and latest
snapshots. Full updates replace prior values; stale/error data is unavailable. Identity used for
local-player selection is discarded. Capture never relies on these messages. Configs, user media
and existing temporary files cannot be overwritten by setup. Local spoofing remains possible;
snapshots are informational, not an authority boundary.

There are no automatic highlights, persisted match annotations or Discord publication yet.
League must pass native TLS/live-match checks on Windows/macOS; Linux portable protocol tests
are not proof of live game support. API and certificate changes can require an application update.
Overwatch/Deadlock have no approved rich adapter without documented supported interfaces.

See [setup, limits and references](../game-integrations.md) and M7.3 in the backlog.
