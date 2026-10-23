# ADR 0023 — Explicit Discord activity disclosure over local IPC

**Status:** accepted for user-requested M10.2 prototype, 2026-10-23; real Discord verification open.

## Context and authorization

The maintainer explicitly asked to implement optional Discord presence with separate game-name
and match-context controls following M7.3. They do not have a Discord Application ID. Complete the
adapter/UI/mock verification without inventing an ID; document the remaining setup requirement.
This authorizes the expressly disclosed activity fields being published via Discord, not media
transfer, product analytics, accounts inside ClipCraft or a required hosted service.

## Decision

Engine `presence.preview` is the only publication policy. Settings default off and separately
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
Without a registered ID no worker probes Discord. Users can explicitly configure their own ID
until a maintainer registers a product application. No counterfeit production ID is shipped.

Mock IPC/portable policy/native UI and compile checks cannot establish actual Discord behavior,
application-name validation, privacy settings, client packaging compatibility or live games.
Those remain release evidence gates. See [user setup and exact fields](../discord-presence.md).
