# ClipCraft identity — CUT

Concept A, selected by the maintainer on 2026-10-24, is the ClipCraft identity. A broad C
opens into a right-facing play shape. The production silhouette is original vector artwork
developed from the AI-assisted concept, with bevelled corners and generous negative space.

## Colour and type

| Role | Colour | Use |
| --- | --- | --- |
| Signal lime | `#C2F84F` | Primary mark on charcoal; accent on paper |
| Charcoal | `#171C1E` | Dark surfaces and single-colour printing |
| Paper | `#F8F7F4` | Light surfaces and reversed mark |

The wordmark reads **ClipCraft**, with both Cs capitalized, in Inter SemiBold. The exported
lockups use the project's existing Inter renderer and font license. Keep the mark larger than
the wordmark's capitals. Use the mark alone for the taskbar, tray-sized placements and Discord
badge; avoid squeezing the full name into those spaces.

## Artwork and spacing

The canonical outline lives in `clipcraft_engine::brand`, shared by native icon rendering,
SVG generation and Discord artwork. The SVG files under `assets/brand/` are reproducible
exports. Do not redraw an independent copy in UI code. No new runtime dependency is needed.

Keep clear space of at least one quarter of the mark's height around the silhouette. Square
icon exports include padding for circular crops. Minimum standalone icon size is 16 pixels;
24 pixels or more is preferred. Use charcoal on pale surfaces for small placements. Lime on
paper is a display accent, not a text colour. Keep the opening clear; avoid outlines, shadows,
gradients, rotation or stretching. Text accompanying the icon remains selectable native UI text.

## Rebuild the kit

```sh
cargo run --release -p clipcraft-engine --example brand_assets -- target/brand-cut-NEW
```

The destination must not exist. The exporter refuses existing destinations and files. It
produces transparent lime/charcoal/paper SVG and 1024-pixel PNG marks, 16–256-pixel icons,
light/dark PNG wordmark lockups, a review board, an opaque 1024-pixel Discord badge, and notices.
Wordmark lockups have explicit backgrounds; the mark PNGs are transparent. Raster exports
are delivery assets; SVG remains the scalable master. Keep generated review boards in ignored
`target/`, and update the committed SVGs together with the canonical outline when it changes.

Upload the CUT PNG as the owned Discord application icon. The presence payload references its
fixed public CDN URL; a separate Rich Presence asset-key upload is unnecessary.
CS2, Dota 2 and League use fixed public Discord CDN icon references in the presence payload.
Discord loads those images; ClipCraft does not download or bundle publisher artwork. Other
games and the offline local preview use CUT. The old typographic game exports are removed.
No Medal or publisher image bytes are part of this kit.

## Provenance

Direction: the maintainer-selected A / CUT concept from the branding exploration. Production
geometry and exports: ClipCraft contributors, MIT. Inter: The Inter Project Authors, SIL OFL
1.1. The generated concept board is a planning reference and is not shipped. See
[`ATTRIBUTION.md`](../ATTRIBUTION.md) for asset-specific entries and licenses.
