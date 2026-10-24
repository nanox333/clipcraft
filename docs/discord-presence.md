# Discord Rich Presence

The `clipcraft` product artwork now uses the selected CUT C/play mark. Generate the complete
identity kit with the [brand exporter](brand.md), or use the existing `discord_assets` exporter
below for the same product badge. Existing game-specific exports remain typographic placeholders.
The application portal must receive the replacement `clipcraft` PNG before Discord can display it.

ClipCraft can optionally publish activity through the **Discord desktop app's local IPC**.
It is off by default, requires no bot/token or login inside ClipCraft, and never affects capture.
The first-party implementation is Rust. There is no ClipCraft server, HTTP client, upload,
Discord SDK runtime, friends UI, join link, or subscription to account/social events.

Unlike local game data, **presence is intended to be visible to other people**: Discord may send
the permitted activity to its service and show it according to your Discord activity/privacy
settings. ClipCraft itself only talks to local Unix sockets or Windows named pipes. Enabling a
game integration does not enable Discord, game-name disclosure or match disclosure.

## End-user setup

Open **Settings → General → Discord Rich Presence** and enable activity. Run the Discord desktop
client with activity sharing enabled. Game-name and match-detail disclosure remain separate,
default-off switches. No Application ID, bot, token, ClipCraft account or browser login is needed
when the build includes the shared ClipCraft identity.

**Current limitation:** no production application has been registered yet. This build says
**Discord presence is not configured in this build yet** and does not probe IPC without an
effective valid ID. The local card preview still works; a preview does not mean connected.

## One-time maintainer setup

1. Create an application named **ClipCraft** at <https://discord.com/developers/applications>.
   Copy its public **Application ID** from General Information. No bot/token/secret is required.
2. Generate original 1024×1024 artwork into a **new** directory:

   ```sh
   cargo run -p clipcraft-engine --example discord_assets -- .clipcraft-local/discord-assets
   ```

   Create the parent directory first if necessary. The exporter refuses an existing destination
   and never overwrites its contents. It includes provenance and the Inter font license.
   Use `clipcraft.png` as the application icon too. Upload
   `clipcraft.png`, `clipcraft_cs2.png`, `clipcraft_dota2.png`, `clipcraft_league.png` in Discord's
   application Rich Presence Art Assets, using each filename stem as its asset key.
3. Set the public repository Actions variable **CLIPCRAFT_DISCORD_APPLICATION_ID** before making
   release packages. The release workflow passes it at compile time; this is **not a secret**.
   For a local build, set the same environment variable when running `cargo build`.
4. Rebuild/install and verify the actual card in Discord, consent reductions, opt-out, exit,
   unavailable-client/reconnect and each target OS. An IPC acknowledgment does not prove that
   uploaded assets rendered. No release/profile verification is implied by local mock tests.

A public custom Application ID remains available under **General → Show advanced settings →
Advanced settings → Discord developer settings**, before the final advanced toggle. **Apply
Application ID** saves the override; **Use shared ClipCraft identity** clears it. A valid saved
override takes priority over the shared build ID. Invalid IDs are ignored; without either valid
identity, nothing connects. No other application's ID is borrowed and no production ID is invented.

## Card appearance

The local preview and outgoing activity use the same disclosure policy and original badge artwork.
With game consent, the card reads **Clipping Counter-Strike 2**, then **with ClipCraft**, with a
large CS2 typographic badge and small ClipCraft badge. Match consent adds readable context, such
as **with ClipCraft · Mirage · Competitive · 8–6**. Dota and League use their own typographic
badges; unknown games use the generic badge. Editing/browsing omit game details and use ClipCraft.
Known internal prefixes (`de_`, `cs_`, `npc_dota_hero_`) are removed from readable labels.

The registered application's heading is **ClipCraft**. This does not spoof Medal's per-game
application heading, publisher artwork, buttons or elapsed timer. Discord controls final layout,
application name, uploaded-asset availability and audience visibility. A local preview is an
approximation of that card, not a screenshot of an actual Discord profile.

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
image URLs or buttons are included. Only fixed uploaded-asset keys and public tooltip labels are
included. The IPC protocol necessarily includes the ClipCraft process ID,
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
objects, runs shared `presence.card` policy (no ID required) and prints the permitted Discord
activity or `null`.
It **never connects or publishes**. JSON stdin is limited to 64 KiB. This is a local CLI command,
not a control server.

Public protocol references reviewed 2026-10-23:
[Discord RPC](https://discord.com/developers/docs/topics/rpc) and
[Discord Rich Presence wire documentation](https://github.com/discord/discord-rpc/blob/master/documentation/hard-mode.md).
Only documented protocol behavior was implemented; no Discord SDK/source/artwork was copied.

Public appearance reference (reviewed 2026-10-23):
[Medal support: Connect Discord to Medal](https://support.medal.tv/support/solutions/articles/48001159956-how-to-connect-your-discord-account-to-medal).
Its public example informed the game/product badge hierarchy and clipping text only. Reference
screenshots are excluded from Git; all shipped/generated artwork is original, with attribution.
