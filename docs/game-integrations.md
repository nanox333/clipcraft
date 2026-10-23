# Local game data

ClipCraft can receive optional match information from **CS2**, **Dota 2**, and **League of Legends**.
This is separate from process-based game recognition and capture arming. Open **Settings → Games
→ Live game data** and enable each game explicitly. Everything is off in existing and new settings.
The application remains entirely Rust; these adapters need no hosted service, account, API key,
plugin, memory reading or injected code. They cannot expose information the game does not provide.

## Setup

- **CS2:** enable its switch, choose **Set up Counter-Strike 2…**, and select the existing
  `Counter-Strike Global Offensive/game/csgo/cfg` directory. Restart CS2 after setup.
- **Dota 2:** enable its switch, choose **Set up Dota 2…**, and select the existing
  `dota 2 beta/game/dota/cfg/gamestate_integration` directory. Create that directory manually if it
  is absent. Add `-gamestateintegration` to Dota's Steam launch options and restart Dota.
- **League:** on Windows/macOS, enable its switch and enter a match. ClipCraft reads the documented
  Live Client Data API at `https://127.0.0.1:2999/liveclientdata/allgamedata`. No config is installed.
  League's integration is unavailable on Linux; its settings row explains the platform requirement.

Valve setup writes only `gamestate_integration_clipcraft.cfg` into the directory you select.
It never modifies other config files or Steam launch options. Repeating setup with identical
contents succeeds; a differing existing config, symlink or occupied temporary is preserved and
reported. Config publication needs a filesystem supporting hard links. The generated config
uses `127.0.0.1:29871` for CS2 and `127.0.0.1:29872` for Dota, with a random authentication token.
Its token is stored separately in `game-state-key` beside ClipCraft's settings, not in diagnostics
or CLI settings output. On Unix, newly created key/config files are owner-readable/writable only.
These are local game-protocol listeners inside ClipCraft, not a separately hosted/control server.

An occupied port or unreadable key shows an error. Disable/re-enable the integration after resolving
it. If the key was lost or damaged, preserve/rename the old key and generated config and set up
again; existing files are never silently replaced. Disabling stops the worker's listener after an
in-flight bounded read completes. The game can retain its config; it cannot deliver updates while
ClipCraft is not listening. Remove this one generated config if you want to undo game setup.

## Fields and behavior

| Game | Selected information |
| --- | --- |
| CS2 | Map/mode, round/phase, team-relative score, local player's health, armour, money, active weapon/ammunition and K/D/A |
| Dota 2 | Game phase/clock, local hero, health/mana, gold and K/D/A |
| League | Map/mode/clock, local champion, health/resource, current gold and K/D/A |

The settings panel shows a live summary and expandable match details. Missing fields are omitted;
an update replaces the previous snapshot rather than carrying old match values forward. Valve
updates expire after 15 seconds without a heartbeat. League polls about once a second when live
and once every five seconds while unavailable; a failed poll clears its live status. Startup,
menus, disconnects and API changes can leave the panel waiting. A rejected Valve update shows a
setup error until the next valid update. A local application can still spoof local match data;
these details must never authorize capture, deletion or another privileged action.

Account names, Steam/Riot identifiers, other-player tables, positions, event histories, tokens
and raw payloads are not retained. Exact identity is used transiently to select the local player.
CS2 spectated-player statistics are omitted; Dota spectator-team objects are not traversed.
The snapshot exists only in memory. **This change does not attach it to saved clips, create
automatic highlight markers/clips.** [Optional Discord presence](discord-presence.md) now selects
map/mode/score/hero through a separate default-off consent policy; enabling game data alone never
publishes anything.

## Local transport and platform limits

Only loopback IPv4 is used. Listeners authenticate Valve payloads, verify the game's AppID and
accept bounded POST requests. Headers are limited to 8 KiB and JSON bodies to 256 KiB, with
bounded IO/deadlines and a maximum four accepted connections per worker turn. One worker retains
one latest snapshot per game, with no growing event queue. Stopping clears the UI snapshot and
releases listeners; failures never change recording settings, capture sources or replay state.

League uses Windows Schannel or macOS Security.framework through Rust `native-tls` bindings.
Normal certificate/hostname verification remains enabled, with Riot's published CA added as a
trust anchor. No certificate bypass, redirects, proxy/environment discovery, DNS, LCU credentials
or Internet connection is used. The dependency is enabled only for Windows/macOS; OpenSSL is not
compiled into ClipCraft on Linux by this integration. See [ADR 0022](adr/0022-local-game-state.md).

CS2/Dota protocol tests run on Linux with synthetic messages and real loopback sockets. League's
parser and HTTP framing are tested portably; native TLS and actual games require live target-OS
checks. macOS match-data bindings do not imply implemented macOS capture. This is prototype
integration support, not verified compatibility with every live game patch or anti-cheat version.

## CLI

The CLI shares the `game_state.install`, `game_state.parse`, and existing recording-settings
commands with the UI:

```sh
clipcraft-cli game-data enable cs2
clipcraft-cli game-data setup cs2 /path/to/game/csgo/cfg
clipcraft-cli game-data enable dota2
clipcraft-cli game-data setup dota2 /path/to/game/dota/cfg/gamestate_integration
clipcraft-cli game-data enable league
clipcraft-cli game-data watch 60
clipcraft-cli game-data disable cs2
clipcraft-cli game-data parse league < local-response.json
```

`watch` runs for 1–3,600 seconds and writes one JSON line per second containing status and selected
snapshot fields. Close the desktop app before using it: only one process can own each Valve port.
`setup` does not implicitly enable a game. `parse` reads at most 256 KiB plus a limit sentinel;
it is an explicit local-file/stdin diagnostic and does not perform transport authentication.

## Other games

No documented local GSI/Live Client equivalent was established for **Overwatch** or **Deadlock**
during this review. Process recognition remains available; richer integration is not promised.
Community web statistics/replay tools do not establish a supported live local endpoint. No
scraping, remote stats dependency, memory reading or anti-cheat hook is added for either game.

Public protocol references (reviewed 2026-10-23):

- [Valve CS GSI documentation](https://developer.valvesoftware.com/wiki/Counter-Strike:_Global_Offensive_Game_State_Integration).
- [Dota2GSI public configuration/schema documentation](https://github.com/antonpup/Dota2GSI#readme),
  consulted as a protocol reference; no implementation was ported.
- [Riot Game Client API](https://developer.riotgames.com/docs/lol#game-client-api), including
  its sample response and [published root CA](https://static.developer.riotgames.com/docs/lol/riotgames.pem).

