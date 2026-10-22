# Automatic game recognition

ClipCraft recognizes 348 known games (all 347 Vice titles plus Minecraft Bedrock) automatically. There is no preset-import step. Settings ›
Games has a global **Recognize known games automatically** switch, a searchable list with per-game
checkboxes. Custom-game add/edit controls are in its bottom **Advanced settings** section,
revealed by the last switch in General; see [settings.md](settings.md). Uncheck an entry to exclude
its built-in identification. Hiding the custom editor does not disable existing custom rules.
Explicit custom rules always win, including custom rules for an excluded built-in game.

This follows Vice's `_match_game` flow: custom definitions first, bundled list second. ClipCraft
adapts Vice's bundled identification data and keeps its game-only capture policy: no recognized game means no
replay capture; closing the game stops it. Exact identifiers and a capturable window are required.
No arbitrary window-title guess or network lookup is used. See [ADR 0015](adr/0015-automatic-known-game-recognition.md).

Older settings missing the new fields use automatic recognition on; their game-only/manual-mode
choice and custom rules remain intact. A recognized running game may arm capture after loading an
older game-only configuration. Explicit global-off and exclusions persist. Merely browsing this
list or changing a game label in the library does not start a manual desktop recording.

## Included games

All titles from [Vice's pinned catalog](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/data/games.json)
are included. Existing ClipCraft CLI IDs and executable/Steam variants are retained; names use
Vice's spelling. Minecraft Bedrock remains an additional ClipCraft entry. The compiled data has
672 process identifiers and 311 exact Steam IDs, with no runtime download or non-Rust build tool.
See [upstream-ports.md](upstream-ports.md) for the source hash and maintainer-reported permission.

| CLI ID | Game |
| --- | --- |
| `cs2` | Counter-Strike 2 |
| `dota2` | Dota 2 |
| `tf2` | Team Fortress 2 |
| `apex` | Apex Legends |
| `fortnite` | Fortnite |
| `valorant` | Valorant |
| `league` | League of Legends |
| `rocket-league` | Rocket League |
| `overwatch2` | Overwatch 2 |
| `pubg` | PUBG: BATTLEGROUNDS |
| `siege` | Tom Clancy's Rainbow Six Siege |
| `rust` | Rust |
| `warframe` | Warframe |
| `destiny2` | Destiny 2 |
| `helldivers2` | Helldivers 2 |
| `dead-by-daylight` | Dead by Daylight |
| `elden-ring` | Elden Ring |
| `baldurs-gate3` | Baldur's Gate 3 |
| `wow` | World of Warcraft |
| `minecraft-bedrock` | Minecraft for Windows (Bedrock) |
| `roblox` | Roblox |
| `minecraft` | Minecraft |
| `final-fantasy-xiv` | Final Fantasy XIV |
| `cyberpunk-2077` | Cyberpunk 2077 |
| `the-witcher-3` | The Witcher 3 |
| `hades-ii` | Hades II |
| `hades` | Hades |
| `hollow-knight` | Hollow Knight |
| `stardew-valley` | Stardew Valley |
| `terraria` | Terraria |
| `factorio` | Factorio |
| `openfront` | OpenFront |
| `rimworld` | Rimworld |
| `risk-of-rain-2` | Risk of Rain 2 |
| `deep-rock-galactic` | Deep Rock Galactic |
| `path-of-exile` | Path of Exile |
| `genshin-impact` | Genshin Impact |
| `old-school-runescape` | Old School RuneScape |
| `forza-horizon-5` | Forza Horizon 5 |
| `war-thunder` | War Thunder |
| `grand-theft-auto-v` | Grand Theft Auto V |
| `fivem` | FiveM |
| `red-dead-redemption-2` | Red Dead Redemption 2 |
| `palworld` | Palworld |
| `black-myth-wukong` | Black Myth: Wukong |
| `monster-hunter-wilds` | Monster Hunter Wilds |
| `monster-hunter-world` | Monster Hunter: World |
| `monster-hunter-rise` | Monster Hunter Rise |
| `hogwarts-legacy` | Hogwarts Legacy |
| `no-man-s-sky` | No Man's Sky |
| `sea-of-thieves` | Sea of Thieves |
| `ark-survival-ascended` | ARK: Survival Ascended |
| `ark-survival-evolved` | ARK: Survival Evolved |
| `hunt-showdown-1896` | Hunt: Showdown 1896 |
| `the-finals` | The Finals |
| `marvel-rivals` | Marvel Rivals |
| `delta-force` | Delta Force |
| `arc-raiders` | ARC Raiders |
| `battlefield-2042` | Battlefield 2042 |
| `battlefield-6` | Battlefield 6 |
| `call-of-duty` | Call of Duty |
| `halo-infinite` | Halo Infinite |
| `halo-the-master-chief-collection` | Halo: The Master Chief Collection |
| `doom-eternal` | DOOM Eternal |
| `dayz` | DayZ |
| `project-zomboid` | Project Zomboid |
| `7-days-to-die` | 7 Days to Die |
| `valheim` | Valheim |
| `satisfactory` | Satisfactory |
| `cities-skylines-ii` | Cities: Skylines II |
| `crusader-kings-iii` | Crusader Kings III |
| `hearts-of-iron-iv` | Hearts of Iron IV |
| `sid-meier-s-civilization-vi` | Sid Meier's Civilization VI |
| `sid-meier-s-civilization-vii` | Sid Meier's Civilization VII |
| `total-war-warhammer-iii` | Total War: Warhammer III |
| `mount-blade-ii-bannerlord` | Mount & Blade II: Bannerlord |
| `age-of-empires-iv` | Age of Empires IV |
| `stellaris` | Stellaris |
| `the-elder-scrolls-v-skyrim` | The Elder Scrolls V: Skyrim |
| `fallout-4` | Fallout 4 |
| `starfield` | Starfield |
| `the-sims-4` | The Sims 4 |
| `among-us` | Among Us |
| `brawlhalla` | Brawlhalla |
| `big-walk` | Big Walk |
| `supermarket-simulator` | Supermarket Simulator |
| `sandustry` | Sandustry |
| `lethal-company` | Lethal Company |
| `phasmophobia` | Phasmophobia |
| `v-rising` | V Rising |
| `don-t-starve-together` | Don't Starve Together |
| `left-4-dead-2` | Left 4 Dead 2 |
| `portal-2` | Portal 2 |
| `slay-the-spire-2` | Slay the Spire 2 |
| `slay-the-spire` | Slay the Spire |
| `balatro` | Balatro |
| `the-binding-of-isaac-rebirth` | The Binding of Isaac: Rebirth |
| `dead-cells` | Dead Cells |
| `celeste` | Celeste |
| `outer-wilds` | Outer Wilds |
| `subnautica` | Subnautica |
| `dave-the-diver` | Dave the Diver |
| `vampire-survivors` | Vampire Survivors |
| `tekken-8` | Tekken 8 |
| `street-fighter-6` | Street Fighter 6 |
| `a-way-out` | A Way Out |
| `abiotic-factor` | Abiotic Factor |
| `against-the-storm` | Against the Storm |
| `age-of-mythology-retold` | Age of Mythology: Retold |
| `alan-wake-2` | Alan Wake 2 |
| `albion-online` | Albion Online |
| `alien-isolation` | Alien: Isolation |
| `american-truck-simulator` | American Truck Simulator |
| `amnesia-the-bunker` | Amnesia: The Bunker |
| `animal-well` | Animal Well |
| `anno-1800` | Anno 1800 |
| `another-crab-s-treasure` | Another Crab's Treasure |
| `armored-core-vi` | Armored Core VI |
| `assetto-corsa` | Assetto Corsa |
| `assetto-corsa-competizione` | Assetto Corsa Competizione |
| `astroneer` | Astroneer |
| `atomic-heart` | Atomic Heart |
| `avowed` | Avowed |
| `backpack-battles` | Backpack Battles |
| `barotrauma` | Barotrauma |
| `battlefield-1` | Battlefield 1 |
| `battlefield-v` | Battlefield V |
| `beamng-drive` | BeamNG.drive |
| `black-desert-online` | Black Desert Online |
| `blue-prince` | Blue Prince |
| `borderlands-3` | Borderlands 3 |
| `borderlands-4` | Borderlands 4 |
| `brotato` | Brotato |
| `buckshot-roulette` | Buckshot Roulette |
| `chained-together` | Chained Together |
| `chivalry-2` | Chivalry 2 |
| `cities-skylines` | Cities: Skylines |
| `clair-obscur-expedition-33` | Clair Obscur: Expedition 33 |
| `clone-drone-in-the-danger-zone` | Clone Drone in The Danger Zone |
| `company-of-heroes-3` | Company of Heroes 3 |
| `content-warning` | Content Warning |
| `core-keeper` | Core Keeper |
| `cult-of-the-lamb` | Cult of the Lamb |
| `cuphead` | Cuphead |
| `dark-souls-ii` | Dark Souls II |
| `dark-souls-iii` | Dark Souls III |
| `dark-souls-remastered` | Dark Souls Remastered |
| `dead-space` | Dead Space |
| `deadlock` | Deadlock |
| `death-must-die` | Death Must Die |
| `deep-rock-galactic-survivor` | Deep Rock Galactic: Survivor |
| `deltarune` | Deltarune |
| `devour` | Devour |
| `diablo-ii-resurrected` | Diablo II: Resurrected |
| `diablo-iv` | Diablo IV |
| `dirt-rally-2-0` | Dirt Rally 2.0 |
| `disco-elysium` | Disco Elysium |
| `divinity-original-sin-2` | Divinity: Original Sin 2 |
| `doom-2016` | DOOM (2016) |
| `doom-the-dark-ages` | DOOM: The Dark Ages |
| `gzdoom` | GZDoom |
| `dragon-age-the-veilguard` | Dragon Age: The Veilguard |
| `dragon-ball-fighterz` | Dragon Ball FighterZ |
| `dragon-ball-sparking-zero` | Dragon Ball Sparking Zero |
| `dragon-s-dogma-2` | Dragon's Dogma 2 |
| `dyson-sphere-program` | Dyson Sphere Program |
| `ea-sports-fc-25` | EA Sports FC 25 |
| `ea-sports-wrc` | EA Sports WRC |
| `elden-ring-nightreign` | Elden Ring Nightreign |
| `empyrion-galactic-survival` | Empyrion: Galactic Survival |
| `enshrouded` | Enshrouded |
| `enter-the-gungeon` | Enter the Gungeon |
| `euro-truck-simulator-2` | Euro Truck Simulator 2 |
| `europa-universalis-iv` | Europa Universalis IV |
| `eve-online` | EVE Online |
| `f1-24` | F1 24 |
| `f1-25` | F1 25 |
| `fall-guys` | Fall Guys |
| `far-cry-5` | Far Cry 5 |
| `far-cry-6` | Far Cry 6 |
| `farming-simulator-25` | Farming Simulator 25 |
| `fatal-fury-city-of-the-wolves` | Fatal Fury: City of the Wolves |
| `final-fantasy-vii-rebirth` | Final Fantasy VII Rebirth |
| `final-fantasy-vii-remake` | Final Fantasy VII Remake |
| `final-fantasy-xvi` | Final Fantasy XVI |
| `five-nights-at-freddy-s-security-breach` | Five Nights at Freddy's: Security Breach |
| `forza-motorsport` | Forza Motorsport |
| `forza-horizon-4` | Forza Horizon 4 |
| `frostpunk` | Frostpunk |
| `frostpunk-2` | Frostpunk 2 |
| `garry-s-mod` | Garry's Mod |
| `gears-of-war-e-day-multiplayer-beta` | Gears of War: E-Day Multiplayer Beta |
| `geoguessr-steam-edition` | GeoGuessr Steam Edition |
| `getting-over-it` | Getting Over It |
| `golf-with-your-friends` | Golf With Your Friends |
| `granblue-fantasy-versus-rising` | Granblue Fantasy Versus: Rising |
| `gray-zone-warfare` | Gray Zone Warfare |
| `green-hell` | Green Hell |
| `grim-dawn` | Grim Dawn |
| `grounded` | Grounded |
| `gtfo` | GTFO |
| `guild-wars-2` | Guild Wars 2 |
| `guilty-gear-strive` | Guilty Gear Strive |
| `half-life-2` | Half-Life 2 |
| `half-life` | Half-Life |
| `black-mesa` | Black Mesa |
| `half-life-alyx` | Half-Life: Alyx |
| `halls-of-torment` | Halls of Torment |
| `hell-let-loose` | Hell Let Loose |
| `hollow-knight-silksong` | Hollow Knight: Silksong |
| `honkai-star-rail` | Honkai: Star Rail |
| `human-fall-flat` | Human Fall Flat |
| `hyper-light-drifter` | Hyper Light Drifter |
| `icarus` | Icarus |
| `injustice-2` | Injustice 2 |
| `inscryption` | Inscryption |
| `insurgency-sandstorm` | Insurgency: Sandstorm |
| `it-takes-two` | It Takes Two |
| `katana-zero` | Katana ZERO |
| `kerbal-space-program` | Kerbal Space Program |
| `kerbal-space-program-2` | Kerbal Space Program 2 |
| `killing-floor-2` | Killing Floor 2 |
| `killing-floor-3` | Killing Floor 3 |
| `kingdom-come-deliverance-ii` | Kingdom Come: Deliverance II |
| `last-epoch` | Last Epoch |
| `left-4-dead` | Left 4 Dead |
| `lies-of-p` | Lies of P |
| `like-a-dragon-infinite-wealth` | Like a Dragon: Infinite Wealth |
| `lords-of-the-fallen` | Lords of the Fallen |
| `lost-ark` | Lost Ark |
| `luck-be-a-landlord` | Luck be a Landlord |
| `madden-nfl-25` | Madden NFL 25 |
| `manor-lords` | Manor Lords |
| `marvel-s-spider-man-remastered` | Marvel's Spider-Man Remastered |
| `mass-effect-legendary-edition` | Mass Effect Legendary Edition |
| `megabonk` | Megabonk |
| `melty-blood-type-lumina` | Melty Blood Type Lumina |
| `metaphor-refantazio` | Metaphor: ReFantazio |
| `metro-exodus` | Metro Exodus |
| `microsoft-flight-simulator-2024` | Microsoft Flight Simulator 2024 |
| `monster-train-2` | Monster Train 2 |
| `mortal-kombat-1` | Mortal Kombat 1 |
| `mortal-kombat-11` | Mortal Kombat 11 |
| `muck` | Muck |
| `multiversus` | MultiVersus |
| `nba-2k25` | NBA 2K25 |
| `necesse` | Necesse |
| `need-for-speed-unbound` | Need for Speed Unbound |
| `neon-white` | Neon White |
| `new-world` | New World |
| `nier-automata` | NieR: Automata |
| `nightingale` | Nightingale |
| `nioh-2` | Nioh 2 |
| `no-rest-for-the-wicked` | No Rest for the Wicked |
| `noita` | Noita |
| `northgard` | Northgard |
| `nuclear-throne` | Nuclear Throne |
| `once-human` | Once Human |
| `ori-and-the-will-of-the-wisps` | Ori and the Will of the Wisps |
| `overcooked-2` | Overcooked 2 |
| `oxygen-not-included` | Oxygen Not Included |
| `paladins` | Paladins |
| `payday-2` | Payday 2 |
| `payday-3` | Payday 3 |
| `peak` | Peak |
| `peglin` | Peglin |
| `persona-3-reload` | Persona 3 Reload |
| `persona-4-golden` | Persona 4 Golden |
| `persona-5-royal` | Persona 5 Royal |
| `pizza-tower` | Pizza Tower |
| `planet-coaster-2` | Planet Coaster 2 |
| `poppy-playtime` | Poppy Playtime |
| `portal` | Portal |
| `prison-architect` | Prison Architect |
| `quake-champions` | Quake Champions |
| `r-e-p-o` | R.E.P.O. |
| `raft` | Raft |
| `ready-or-not` | Ready or Not |
| `rematch` | Rematch |
| `remnant-ii` | Remnant II |
| `resident-evil-2` | Resident Evil 2 |
| `resident-evil-4` | Resident Evil 4 |
| `resident-evil-village` | Resident Evil Village |
| `rivals-of-aether-ii` | Rivals of Aether II |
| `rogue-legacy-2` | Rogue Legacy 2 |
| `schedule-i` | Schedule I |
| `scum` | SCUM |
| `sekiro-shadows-die-twice` | Sekiro: Shadows Die Twice |
| `shapez-2` | Shapez 2 |
| `sid-meier-s-civilization-v` | Sid Meier's Civilization V |
| `silent-hill-2` | Silent Hill 2 |
| `skullgirls-2nd-encore` | Skullgirls 2nd Encore |
| `slime-rancher-2` | Slime Rancher 2 |
| `smite-2` | Smite 2 |
| `sniper-elite-5` | Sniper Elite 5 |
| `songs-of-conquest` | Songs of Conquest |
| `sonic-frontiers` | Sonic Frontiers |
| `sonic-x-shadow-generations` | Sonic X Shadow Generations |
| `sons-of-the-forest` | Sons of the Forest |
| `soulcalibur-vi` | Soulcalibur VI |
| `space-engineers` | Space Engineers |
| `split-fiction` | Split Fiction |
| `splitgate-2` | Splitgate 2 |
| `squad` | Squad |
| `star-wars-battlefront-ii` | Star Wars Battlefront II |
| `star-wars-the-old-republic` | Star Wars: The Old Republic |
| `starcraft-ii` | StarCraft II |
| `star-citizen` | Star Citizen |
| `stellar-blade` | Stellar Blade |
| `stray` | Stray |
| `street-fighter-v` | Street Fighter V |
| `tekken-7` | Tekken 7 |
| `the-elder-scrolls-online` | The Elder Scrolls Online |
| `the-first-berserker-khazan` | The First Berserker Khazan |
| `the-forest` | The Forest |
| `the-isle` | The Isle |
| `the-king-of-fighters-xv` | The King of Fighters XV |
| `the-outlast-trials` | The Outlast Trials |
| `the-planet-crafter` | The Planet Crafter |
| `they-are-billions` | They Are Billions |
| `throne-and-liberty` | Throne and Liberty |
| `timberborn` | Timberborn |
| `titanfall-2` | Titanfall 2 |
| `trackmania` | Trackmania |
| `trailmakers` | Trailmakers |
| `two-point-museum` | Two Point Museum |
| `ultrakill` | Ultrakill |
| `under-night-in-birth-ii` | Under Night In-Birth II |
| `undertale` | Undertale |
| `unravel-two` | Unravel Two |
| `unreal-2-the-awakening` | Unreal 2: The Awakening |
| `unreal-gold` | Unreal Gold |
| `unreal-tournament` | Unreal Tournament |
| `unreal-tournament-2004` | Unreal Tournament 2004 |
| `unreal-tournament-3` | Unreal Tournament 3 |
| `unturned` | Unturned |
| `victoria-3` | Victoria 3 |
| `warhammer-40-000-darktide` | Warhammer 40,000: Darktide |
| `warhammer-40-000-rogue-trader` | Warhammer 40,000: Rogue Trader |
| `warhammer-40-000-space-marine-2` | Warhammer 40,000: Space Marine 2 |
| `warhammer-vermintide-2` | Warhammer: Vermintide 2 |
| `webfishing` | Webfishing |
| `wo-long-fallen-dynasty` | Wo Long: Fallen Dynasty |
| `wolfenstein-ii` | Wolfenstein II |
| `wreckfest` | Wreckfest |
| `wuthering-waves` | Wuthering Waves |
| `yakuza-like-a-dragon` | Yakuza: Like a Dragon |
| `zenless-zone-zero` | Zenless Zone Zero |

The converted data lives in
[`game_catalog_data.rs`](../crates/clipcraft-engine/src/game_catalog_data.rs); matching and exclusion
validation live in [`game_catalog.rs`](../crates/clipcraft-engine/src/game_catalog.rs).
These are identification candidates, not a tested capture compatibility list.

## Matching and limits

- Built-ins use case-insensitive **whole executable basenames**, such as `Wow.exe`, to avoid
  substring lookalikes and launcher/helper processes. Custom rules expose an **Exact** switch;
  existing custom rules keep their prior substring matching unless the user changes it.
- Custom rules are checked before built-ins. Within either layer, exact Steam IDs take precedence
  over executable matches. Linux can read numeric Steam IDs from process metadata; verified local
  installation paths supply IDs on other platforms too. IDs do not establish capture, Proton or
  anti-cheat compatibility.
- Generic runtimes/launchers (`java.exe`, `javaw.exe`, Wine, Steam, Epic, Riot clients) are not
  catalog executable rules. Minecraft is listed via its `minecraft` identifier, but a generic Java
  process alone cannot identify it. Vice window-class-only aliases cannot identify a process with
  a different basename in ClipCraft.
- Vice uses substring matching for its defaults; ClipCraft deliberately keeps built-in aliases
  case-insensitive and exact. Seven aliases are omitted to avoid launcher/generic-process arming:
  `minecraft-launcher`, `PrismLauncher`, `tlauncher`, `leagueclient.exe`, `fortnitelauncher`,
  `PlayGTAV.exe`, and `Client-Win64-Shipping.exe`. Their titles remain included with other identifiers.
  No executable is inferred from a game's title or a partially matching process name.
- Exclusion commands accept at most 1,024 IDs, enough to disable the entire catalog; settings use
  the same bound and remain subject to the 64 KiB file limit. Custom rules retain their separate
  64-rule limit. Catalog size is checked against the exclusion budget in tests.
- Game versions/storefronts can change executables. Add a custom override when needed. Custom
  rules remain unchanged by catalog updates, and exclusions use stable game IDs. A catalog update
  may add identification variants for an enabled known game; excluded entries remain excluded.
- Detection still needs a capturable game window. Wayland process-to-window matching, macOS
  capture, protected games and real-game evidence remain subject to the existing platform gaps.
- Recognition does not retroactively tag existing videos. Newly saved game-managed clips use
  the matched custom or built-in name through the captured-game metadata path.
- Local Steam manifests now supply names and exact IDs beyond the bundled list. Vice-style
  window-class/compositor matching remains unavailable; Steam metadata does not bypass the need
  for a capturable window. Exited zombie processes are ignored when deciding whether a game runs.

## Local CLI

```sh
clipcraft-cli recording games list
clipcraft-cli recording games installed
clipcraft-cli recording games installed /absolute/path/to/Steam
clipcraft-cli recording games disable cs2 wow
clipcraft-cli recording games enable cs2
clipcraft-cli recording games automatic off
clipcraft-cli recording games automatic on
clipcraft-cli recording settings
```

`list` prints the shipped metadata. `enable`/`disable` change built-in exclusions through the same
`recording.set_builtin_games` engine command as the UI; `all` may replace the ID list. `automatic`
sets the global built-in switch. Custom rules and the game-only/manual-mode choice are preserved.
CLI changes apply when preferences are next loaded; no control server is used. `installed` reads
local Steam installation metadata through `recording.scan_steam_games`; it accepts up to 64
explicit absolute roots, or uses platform defaults. It does not inspect game assets, process
memory, a network service or an account.

Generated matching tests check every executable variant and Steam ID, case handling, launcher
lookalikes, custom-rule preservation, custom precedence, malformed/duplicate exclusions, global-off, migration and
save/reload behavior. These tests verify matching behavior, not a live capture run of those games.

## Larger database considered

[leinstay/steamdb](https://github.com/leinstay/steamdb) was inspected at
`6ade3eeaf15d6ac3de2de32cb92eb59c71b6f4bd` (2026-10-22). Its README reports 194,282
catalog rows, a 353 MB minified JSON dump and Steam IDs/titles, but no executable-identification
fields. It declares GPL-3.0 and aggregates third-party metadata. It is not bundled or queried;
the permission reported for Vice/ClipRecorder does not establish permission for this separate
source. No store artwork, descriptions, prices or publisher assets are imported. The local
Steam adapter below resolves installed IDs and names without this database.


## Installed Steam discovery and automatic switching

**Recognize installed Steam games** is enabled under automatic recognition. One background worker
reads local `steamapps/libraryfolders.vdf` and `appmanifest_*.acf` files, caches the catalog for
30 seconds and refreshes it on request. It also runs process matching and the running-app picker;
UI rendering does not enumerate processes or parse manifests. One pending request replaces older
requests, and results from obsolete settings cannot start recording.

Custom rules take priority, followed by enabled bundled matches, then installed Steam IDs. The
global recognition switch and both bundled and installed-ID exclusions remain effective. Verified
executable paths beneath installed game directories can supply Steam IDs locally; full process
paths are not returned or stored. Generic launchers/crash helpers are excluded from this inference.
Linux Steam process IDs take precedence over path inference. Discovery checks standard Windows
Steam roots, Linux native/Flatpak roots and the macOS default root; custom Windows installations
are not located through the registry. macOS capture itself remains unavailable.

Parsing is bounded to 64 roots/libraries, 4,096 entries, 1,024 games, 12 nesting levels and 100,000
tokens per metadata file. Library indexes are at most 1 MiB; manifests at most 64 KiB. Install
folders must exist beneath the library's `common` directory, manifest IDs must agree with their
filenames, and individual manifest symlinks are skipped. Invalid manifests are counted and skipped;
an invalid library index or exceeded whole-scan budget returns an error and keeps the prior catalog.
Filesystem calls already blocked on an unavailable drive cannot be interrupted.

See [settings.md](settings.md#automatic-recording-behavior) for **Games only**,
timed desktop continuation, continuous desktop/game switching, and manual pause behavior.
