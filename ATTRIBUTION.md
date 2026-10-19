# Asset attribution

| Asset | Source | License / attribution |
| --- | --- | --- |
| egui default UI fonts (transitive `epaint_default_fonts` dependency) | egui project | OFL-1.1 and Ubuntu Font Licence 1.0; redistributed under the package's included license files |
| `assets/fonts/Inter-Regular.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-Medium.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |
| `assets/fonts/Inter-SemiBold.ttf` | Inter 4.1, The Inter Project Authors, <https://github.com/rsms/inter/releases/tag/v4.1> | SIL OFL 1.1, unmodified; text in `assets/fonts/OFL-Inter.md` |

Add every first-party shipped asset to this table and place a `<filename>.license.md` sidecar beside
it with its source, author, and license. `cargo xtask assets` checks that each file under `assets/`
has a sidecar and a matching path in this table. User-provided Medal screenshots and downloaded
reference images are private planning references, not product assets.
