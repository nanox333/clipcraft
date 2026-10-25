# Discord Rich Presence

The `clipcraft` product artwork now uses the selected CUT C/play mark. Generate the complete
identity kit with the [brand exporter](brand.md), or use the existing `discord_assets` exporter
below for the same product badge. Game placeholders are removed: CS2, Dota 2 and League use
fixed Discord-hosted icon references. ClipCraft never downloads or bundles those image files.
The original CUT icon is uploaded to the owned application; its fixed CDN reference supplies the product badge too.

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

The maintainer supplied public application ID `1563496212554846279` on 2026-10-24. Ordinary builds
now include it. Previously, a missing build environment variable left the identity unset even
when presence was enabled. An empty release variable now also uses the product default. A local
card preview still does not prove a connection or actual Discord profile visibility. Artwork
upload and live rendering remain separate checks.

A Windows IPC read bug is also fixed: an empty connected byte pipe now waits for data instead of
being treated as a disconnect before Discord's reply arrives. An explicit live Windows check on
2026-10-24 received READY and an activity acknowledgment using the shared ID, then requested clear
and disconnected. This verifies transport acceptance, not the final profile's artwork or audience.

On 2026-10-24 the CUT application icon and **ClipCraft** capitalization were saved in the portal.
The real Windows profile initially hid acknowledged activity because Discord's **Activity
Privacy → Share my activity** switch was off. After the maintainer enabled it, CS2, Dota 2 and
League cards rendered their Discord-hosted icons with the small CUT badge. This controls Discord's broader
game/connected-app activity sharing; ClipCraft cannot change it. The application heading can
retain Discord's cached former capitalization until the client refreshes its application metadata.

## One-time maintainer setup

1. Create an application named **ClipCraft** at <https://discord.com/developers/applications>.
   Copy its public **Application ID** from General Information. No bot/token/secret is required.
2. Generate original 1024×1024 artwork into a **new** directory:

   ```sh
   cargo run -p clipcraft-engine --example discord_assets -- .clipcraft-local/discord-assets
   ```

   Create the parent directory first if necessary. The exporter refuses an existing destination
   and never overwrites its contents. It includes provenance and the MIT license.
   Upload `clipcraft.png` as the application icon in General Information. Record its public
   CDN URL in `PRODUCT_ICON_URL` when replacing the branded icon. The current CUT icon is
   already uploaded. Separate Rich Presence art-key and publisher-image uploads are unnecessary.
3. ClipCraft's shared public Application ID is included in the Rust source. Optionally set the
   repository Actions variable **CLIPCRAFT_DISCORD_APPLICATION_ID** to override it for release
   packages, or the same environment variable for a local build. This is **not a secret**.
   Missing or empty build values use the product default; an invalid nonempty override disables
   the shared identity until corrected. A valid saved developer ID still takes priority.
4. Rebuild/install and verify the actual card in Discord, consent reductions, opt-out, exit,
   unavailable-client/reconnect and each target OS. An IPC acknowledgment does not prove that
   uploaded assets rendered. No release/profile verification is implied by local mock tests.

A public custom Application ID remains available under **General → Show advanced settings →
Advanced settings → Discord developer settings**, before the final advanced toggle. **Apply
Application ID** saves the override; **Use shared ClipCraft identity** clears it. A valid saved
override takes priority over the shared build ID. Invalid IDs are ignored; without either valid
identity, nothing connects. No other application's ID is borrowed and no production ID is invented.

## Card appearance

The local preview and outgoing activity use the same disclosure policy.
With game consent, the card reads **Clipping Counter-Strike 2**, then **with ClipCraft**, with a
large Discord-hosted CS2 icon and small CUT badge. Match consent adds readable context, such
as **with ClipCraft · Mirage · Competitive · 8–6**. Dota and League use their own Discord-hosted
icons; other games use CUT. Editing/browsing omit game details and use ClipCraft.
The local preview always uses offline CUT artwork and explicitly labels that difference.
Known internal prefixes (`de_`, `cs_`, `npc_dota_hero_`) are removed from readable labels.

The registered application's heading is **ClipCraft**. This does not spoof Medal's per-game
application heading, publisher artwork, buttons or elapsed timer. Discord controls final layout,
application name, uploaded-asset availability and audience visibility. A local preview is an
approximation of that card, not a screenshot of an actual Discord profile.

## Exactly what is published

| Control | Fields permitted |
| --- | --- |
| Show ClipCraft activity on Discord | One activity label: `Clipping`, `Editing clips`, or `Browsing clips`, with original CUT product art |
| Show the captured game's name | Additionally, the current captured game's label and a fixed CDN icon for supported games, only while clipping |
| Show map, mode, score and hero | Additionally, available map, mode, team-relative score and hero/champion from the matching fresh CS2/Dota/League adapter |

The second and third switches default off. Match context additionally requires game-name consent.
Editing takes priority over clipping for the basic label and never includes a saved clip's game,
title or path. Manual desktop capture cannot infer a game from stale integration data. Unknown
custom game labels can be shown with game-name consent, but cannot claim another game's match
snapshot. Current matching recognizes CS2/Counter-Strike 2, Dota 2/Dota2 and League of Legends.

No clip/media bytes, filenames/paths, clip titles, local library metadata, account identifiers,
K/D/A, health, money, positions, match IDs, raw game payloads, credentials, timestamps, join secrets,
arbitrary image URLs or buttons are included. Only four fixed public Discord CDN references
(three games and the original CUT icon) and public tooltip labels are included.
Game-name consent gates both the name and game-icon reference. The IPC protocol includes the ClipCraft process ID,
Application ID and a local command nonce. Ready/ack frames may contain Discord user data; the
adapter bounds/parses them only to identify protocol responses, then discards them without logging
or storing account fields. It does not request authentication or subscribe to events.

### Game-icon provenance and limits

On 2026-10-24 the three application IDs/icon hashes were checked using public, unauthenticated
Discord application metadata. They identify artwork only; the IPC handshake always uses the
owned ClipCraft application ID. The product does not call Discord discovery or metadata APIs,
download publisher images, introduce an HTTP client, or store/cache remote artwork. Core capture
and editing remain offline. Discord's client/service may fetch and cache CDN images as part of
enabled presence. Fixed hashes can become stale; updating them is a maintainer data change.
CDN failure cannot be detected through an activity acknowledgment, so the visual check is separate.

Vice's public [game_icons.py](https://github.com/splitshotz/Vice/blob/main/vice/game_icons.py)
and presence construction informed the behavior; no Vice source is copied. Unlike Vice's runtime
lookup, ClipCraft uses only three fixed references and no discovery API in the product.
Discord documents external image references in its
[ActivityAssets reference](https://discord.com/developers/docs/social-sdk/classdiscordpp_1_1ActivityAssets.html).
This avoids redistributing publisher image files; it is not a claim to own or relicense their marks.

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
Windows has a basic live activity acknowledgment with the configured ID, plus deterministic
empty/data/disconnect pipe coverage. Actual Discord profile artwork/audience, macOS runtime and
live-game verification remain manual gates. See [testing](testing.md) and [ADR 0023](adr/0023-discord-presence.md).

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
