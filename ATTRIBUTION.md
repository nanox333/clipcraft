# Asset attribution

The bundled microphone noise-reduction model (`src/weights.rnn`, 87,521 bytes) is distributed
inside the unmodified `nnnoiseless` 0.5.2 dependency under BSD-3-Clause. Copyright holders include
Joe Neeman, Mozilla, Jean-Marc Valin, Xiph.Org and Mark Borgerding. The required notice is retained
in [nnnoiseless license](docs/licenses/nnnoiseless-BSD-3-Clause.txt); see [ADR 0028](docs/adr/0028-microphone-noise-suppression.md).

`crates/clipcraft-platform/src/riotgames.pem` is the unmodified public Riot Games Game Client API
root certificate from <https://static.developer.riotgames.com/docs/lol/riotgames.pem>, retrieved
2026-10-23. SHA-256: `da884275737f024b33c93ae5d28bdb002768a3cb73752ab40254a32218193521`.
It is public cryptographic trust material (certificate/key facts), not game art, publisher source
code or a creative asset under an asserted license. No Riot artwork or SDK is included.
See [ADR 0022](docs/adr/0022-local-game-state.md) for the narrowly scoped trust use.

| Asset | Source | License / attribution |
| --- | --- | --- |
| `assets/brand/mark-lime.svg` | Original ClipCraft CUT geometry, developed from the maintainer-selected AI-assisted concept A; ClipCraft contributors | MIT; no third-party artwork or embedded font |
| `assets/brand/mark-black.svg` | Same original CUT geometry, charcoal variant | MIT |
| `assets/brand/mark-white.svg` | Same original CUT geometry, paper variant | MIT |
| CUT raster exports (`brand_assets`, `presence_art` product key `clipcraft`) | Original ClipCraft C/play geometry; generated in Rust. Wordmark lockups use Inter SemiBold | MIT (ClipCraft); Inter glyphs remain SIL OFL 1.1, included in exporter output |
| egui default UI fonts (transitive `epaint_default_fonts` dependency) | egui project | OFL-1.1 and Ubuntu Font Licence 1.0; redistributed under the package's included license files |
| `assets/fonts/Inter-Regular.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-Medium.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-SemiBold.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/icons/chevron-down.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/chevron-left.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/copy.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/exit-fullscreen.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/folder.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/fullscreen.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/gear.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/heart.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/import.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/library.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/loop.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/pause.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/play.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/plus.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/record.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/restart.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/scissors.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/search.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/skip-back.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/skip-forward.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/speaker.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/speaker-off.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/step-back.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/step-forward.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/stop.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |
| `assets/icons/trash.svg` | Original artwork drawn for ClipCraft | Project license; no third-party source |

Add every first-party shipped asset to this table and place a `<filename>.license.md` sidecar beside
it with its source, author, and license. `cargo xtask assets` checks that each file under `assets/`
has a sidecar and a matching path in this table. User-provided Medal screenshots and downloaded
reference images are private planning references, not product assets. Source-code ports are tracked
separately in [`docs/upstream-ports.md`](docs/upstream-ports.md).

Discord game artwork is referenced by fixed public CDN URLs in `presence::game_icon_url`, not
redistributed as bundled media or relicensed as ClipCraft artwork. The CS2, Dota 2 and League
application IDs/icon hashes were checked against public Discord application metadata on 2026-10-24.
Discord fetches the referenced images after game-name consent. See [presence](docs/discord-presence.md)
for provenance, limitations and the absence of runtime metadata lookups.

The compiled identification metadata in
[`game_catalog_data.rs`](crates/clipcraft-engine/src/game_catalog_data.rs) adapts Vice's 347-game
catalog at `7e4b2a3`, with ClipCraft variants and one Bedrock entry. It contains names/identifiers,
not publisher artwork. This is the maintainer-requested catalog reuse under reported author
permission; Vice's project license is GPL-3.0-or-later, not a claimed permissive grant. Exact
source, hash, omitted aliases and permission provenance are in
[`docs/upstream-ports.md`](docs/upstream-ports.md).
