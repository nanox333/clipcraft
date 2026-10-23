# Asset attribution

`crates/clipcraft-platform/src/riotgames.pem` is the unmodified public Riot Games Game Client API
root certificate from <https://static.developer.riotgames.com/docs/lol/riotgames.pem>, retrieved
2026-10-23. SHA-256: `da884275737f024b33c93ae5d28bdb002768a3cb73752ab40254a32218193521`.
It is public cryptographic trust material (certificate/key facts), not game art, publisher source
code or a creative asset under an asserted license. No Riot artwork or SDK is included.
See [ADR 0022](docs/adr/0022-local-game-state.md) for the narrowly scoped trust use.

| Asset | Source | License / attribution |
| --- | --- | --- |
| egui default UI fonts (transitive `epaint_default_fonts` dependency) | egui project | OFL-1.1 and Ubuntu Font Licence 1.0; redistributed under the package's included license files |
| `assets/fonts/Inter-Regular.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-Medium.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-SemiBold.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |

Add every first-party shipped asset to this table and place a `<filename>.license.md` sidecar beside
it with its source, author, and license. `cargo xtask assets` checks that each file under `assets/`
has a sidecar and a matching path in this table. User-provided Medal screenshots and downloaded
reference images are private planning references, not product assets. Source-code ports are tracked
separately in [`docs/upstream-ports.md`](docs/upstream-ports.md).

The compiled identification metadata in
[`game_catalog_data.rs`](crates/clipcraft-engine/src/game_catalog_data.rs) adapts Vice's 347-game
catalog at `7e4b2a3`, with ClipCraft variants and one Bedrock entry. It contains names/identifiers,
not publisher artwork. This is the maintainer-requested catalog reuse under reported author
permission; Vice's project license is GPL-3.0-or-later, not a claimed permissive grant. Exact
source, hash, omitted aliases and permission provenance are in
[`docs/upstream-ports.md`](docs/upstream-ports.md).
