# ADR 0023 — Explicit Discord activity disclosure over local IPC

**Status:** accepted for user-requested M10.2 prototype, 2026-10-23; real Discord verification open.

## Context and authorization

The maintainer explicitly asked to implement optional Discord presence with separate game-name
and match-context controls following M7.3. They do not have a Discord Application ID. Complete the
adapter/UI/mock verification without inventing an ID; document the remaining setup requirement.
This authorizes the expressly disclosed activity fields being published via Discord, not media
transfer, product analytics, accounts inside ClipCraft or a required hosted service.

## Decision

Engine `presence.preview` builds IPC targets; `presence.card` exposes the same publication policy
for an identity-independent preview. Settings default off and separately
authorize basic activity, current captured game name, and selected map/mode/team score/hero context.
Its fixed allowlist excludes all media, paths/titles, account fields, raw game data and other stats.
Only fresh matching integration snapshots while capturing a recognized game can enrich presence;
editing/browsing and desktop fallback cannot inherit a saved clip's game or previous match.

One Rust platform worker uses Discord's documented handshake/READY/SET_ACTIVITY/nonce/ping protocol.
No SDK, Discord account API, authentication, subscriptions, join/spectate capability, TCP client,
or backend is added. Capture is independent. Bounded IO and frames, a single latest target, five-
second update/retry intervals and clearing on privacy reductions bound resources and exposure.
Normal exit requests a bounded clear; disconnect is the fallback for an unavailable peer.

Use Rust `socket2` 0.6 (MIT OR Apache-2.0), locked to 0.6.5, for safe bounded Unix-domain connects.
Use Windows-only `interprocess` pinned 2.4.6 (0BSD) for safe nonblocking local named-pipe bindings.
These are platform adapters, with no first-party unsafe or bundled non-Rust runtime. Windows pipe
cleanup calls `assume_flushed` after the bounded protocol exchange so library drop cannot create
unbounded flush/limbo work against an unresponsive peer; an acknowledged clear is preferred, and
unsent activity is allowed to be lost on disconnect. Dependencies, licenses and layers are checked
by the normal gate. No upstream implementation source or Discord artwork is ported.

## Consequences

Discord may publish the allowed activity over its own service; local-only refers to ClipCraft's
transport, not to final visibility. Explain fields before opting in and defer audience controls
to Discord. Keep the public Application ID separate from tokens: no token/secret is accepted.
Without an effective valid ID no worker probes Discord. M10.2a adds a shared compile-time public
`CLIPCRAFT_DISCORD_APPLICATION_ID` supplied once by the maintainer/release repository variable.
A valid saved developer override takes priority; it is hidden in General's advanced section.
On 2026-10-24 the maintainer supplied public application ID `1563496212554846279`. It is now the
product default in Rust source so ordinary builds and empty release variables work without extra
setup. Nonempty build overrides still require validation; malformed values do not silently select
another identity. Artwork upload and real Discord rendering remain separate verification gates.

Medal's public support example informed the card hierarchy: game badge, small product badge,
`Clipping <game>` and `with ClipCraft`. This is independent original art generated in bounded Rust
with existing Inter glyphs; no publisher/Medal logos or screenshot pixels are shipped. Fixed asset
keys are uploaded once to the owned application. The default header is the registered **ClipCraft**
application name; this does not promise Medal's per-game header or arbitrary identity overrides.
No elapsed timestamp is added. Readable map/hero labels remove known internal prefixes. Asset
selection obeys game-name consent. The local preview uses the same generated art and engine policy.
Missing uploaded assets cannot be detected by local IPC ACK; real rendering remains a manual gate.

Windows byte-pipe reads first call `PeekNamedPipe` through one platform-only unsafe block. The
handle is borrowed from the owned OVERLAPPED stream and only an initialized byte-count output is
passed. The worker is the sole reader. An empty connected pipe returns `WouldBlock`; a broken
pipe returns an error and takes the existing bounded reconnect path. This prevents the byte-stream
dependency's empty nonblocking read from being mistaken for EOF before READY or an acknowledgment
arrives. Reads are capped to both available bytes and the caller's bounded buffer. A local pipe
regression covers empty/data/empty/disconnect transitions. No other layer receives a native handle.
Reference: [PeekNamedPipe](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-peeknamedpipe).

Mock IPC/portable policy/native UI and compile checks cannot establish actual Discord behavior,
application-name validation, privacy settings, client packaging compatibility or live games.
Those remain release evidence gates. See [user setup and exact fields](../discord-presence.md).
