# Discord Rich Presence

ClipCraft can optionally publish activity through the **Discord desktop app's local IPC**.
It is off by default, requires no bot/token or login inside ClipCraft, and never affects capture.
The first-party implementation is Rust. There is no ClipCraft server, HTTP client, upload,
Discord SDK runtime, friends UI, join link, or subscription to account/social events.

Unlike local game data, **presence is intended to be visible to other people**: Discord may send
the permitted activity to its service and show it according to your Discord activity/privacy
settings. ClipCraft itself only talks to local Unix sockets or Windows named pipes. Enabling a
game integration does not enable Discord, game-name disclosure or match disclosure.

## First-time setup

No ClipCraft Discord Application ID has been registered/bundled yet. Until one is supplied, the
feature says **Add a Discord Application ID to connect** and performs no IPC discovery.

1. Open <https://discord.com/developers/applications> in your browser and create an application
   named **ClipCraft**. This step uses your Discord account in Discord's portal, not in ClipCraft.
2. Under **General Information**, copy the **Application ID** (also called Client ID).
   Do not create a bot or copy a bot token/client secret; neither is required.
3. Open **Settings → General → Discord Rich Presence**, enable activity, paste the ID, then
   choose **Apply Application ID**. The name displayed by Discord comes from that application.
4. Run the Discord desktop client with activity sharing enabled in Discord. The browser client
   does not expose the local IPC endpoint. A real client must accept the ID/activity before the
   status becomes **Connected to Discord**.

The ID is public identification, not a credential, and is saved with local recording settings.
Invalid IDs are not used; there is no fallback to someone else's application or a fabricated ID.
The test IDs used in fixtures cannot establish a production Discord identity. A future maintainer
may register an application and provide its ID as the product default in a separate change.

## Exactly what is published

| Control | Fields permitted |
| --- | --- |
| Show ClipCraft activity on Discord | One activity label: `Clipping`, `Editing clips`, or `Browsing clips` |
| Show the captured game's name | Additionally, the current captured game's label, only while clipping |
| Show map, mode, score and hero | Additionally, available map, mode, team-relative score and hero/champion from the matching fresh CS2/Dota/League adapter |

The second and third switches default off. Match context additionally requires game-name consent.
Editing takes priority over clipping for the basic label and never includes a saved clip's game,
title or path. Manual desktop capture cannot infer a game from stale integration data. Unknown
custom game labels can be shown with game-name consent, but cannot claim another game's match
snapshot. Current matching recognizes CS2/Counter-Strike 2, Dota 2/Dota2 and League of Legends.

No clip/media bytes, filenames/paths, clip titles, local library metadata, account identifiers,
K/D/A, health, money, positions, match IDs, raw game payloads, credentials, timestamps, join secrets,
images or buttons are included. The IPC protocol necessarily includes the ClipCraft process ID,
Application ID and a local command nonce. Ready/ack frames may contain Discord user data; the
adapter bounds/parses them only to identify protocol responses, then discards them without logging
or storing account fields. It does not request authentication or subscribe to events.

## Updates, failure and clearing

One replaceable target keeps the latest state; unchanged activity is not resent. Routine activity
updates are spaced at least five seconds apart. Ping/pong and acknowledgments are handled locally.
Disabling, changing the ID, removing a game, losing fresh match data or reducing consent clears the
old activity first, even during that update interval. Publishing a less detailed replacement may
then take up to the normal interval. Game closure follows the existing process-observation cadence;
integration staleness follows the [local game-data rules](game-integrations.md).

Discord missing, busy, closed or disconnected shows a waiting status and retries about every five
seconds while enabled. Rejection says to check the ID and Discord settings. A worker failure asks
for disable/re-enable. None of these states can start/stop capture or alter replay/media files.

Turning presence off sends `SET_ACTIVITY` with `activity: null`, then disconnects. Normal app exit
requests the same clear, with at most 1.2 seconds spent waiting for worker completion. Frames/IO
are bounded and failed peers are disconnected. Clearing is best effort when Discord is unavailable,
unresponsive, or the application is killed: Discord owns final profile visibility and removal on
IPC disconnect. No client can guarantee a remote profile update while that service is unreachable.

## Platform and protocol limits

Linux/macOS use local `discord-ipc-0`…`9` Unix sockets under standard runtime/temporary roots,
including common Flatpak/Snap subdirectories. Windows uses only local `\\.\pipe\discord-ipc-N`.
The worker never opens a TCP connection, browser control service, remote pipe hostname or proxy.
Sandbox permissions or unsupported packaging may prevent access; no permission bypass is attempted.

Incoming/outgoing frames are limited to 16 KiB, little-endian opcode/length framing. IO waits are
bounded to 500 ms per exchange and connection attempts do not wait indefinitely. Nonblocking work
and per-turn frame limits prevent unbounded buffering/draining. Malformed/oversized/truncated data,
unknown opcodes, close/error events, missing READY or mismatched/missing nonce acknowledgments
cannot report success. No raw Discord error message is copied into diagnostics.

Portable policy and Unix mock IPC tests cover handshake, publish/clear, ping/pong, rejection,
coalescing, rate limiting, privacy reductions, reconnect, late READY after opt-out and ID changes.
Windows/macOS compile checks are not runtime proof. A registered ID, actual Discord profile check
and live-game verification remain manual gates. See [testing](testing.md) and [ADR 0023](adr/0023-discord-presence.md).

## CLI

```sh
clipcraft-cli presence settings
clipcraft-cli presence configure < presence-settings.json
clipcraft-cli presence preview < presence-preview.json
```

`configure` takes the complete presence-settings object (`enabled`, `application_id`, `show_game`,
`show_match`), defaulting omitted values, and saves it through the existing engine settings command.
Restart an already-running desktop app to load CLI changes. `preview` takes `settings` and `context`
objects, runs shared `presence.preview` policy and prints the permitted Discord activity or `null`.
It **never connects or publishes**. JSON stdin is limited to 64 KiB. This is a local CLI command,
not a control server.

Public protocol references reviewed 2026-10-23:
[Discord RPC](https://discord.com/developers/docs/topics/rpc) and
[Discord Rich Presence wire documentation](https://github.com/discord/discord-rpc/blob/master/documentation/hard-mode.md).
Only documented protocol behavior was implemented; no Discord SDK/source/artwork was copied.
