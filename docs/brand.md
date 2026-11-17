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

## At a glance

| | |
| --- | --- |
| Name | **ClipCraft**, one word, both Cs capital. Never "Clipcraft", "Clip Craft" or "CLIPCRAFT" in text |
| Mark | **CUT**: a broad C opening into a right-facing play shape |
| Colours | signal lime, charcoal, paper (table above); the app adds the greys and message colours below |
| Type | Inter (Regular, Medium, SemiBold), bundled under the SIL Open Font License |
| Icons | original line icons in `assets/icons/`, drawn on a 24-unit square |
| Feel | calm, dark, one lime action at a time; no gradients on the mark, no decoration for its own sake |

## Using the mark

Where the mark appears today: the window and taskbar icon, the tray icon, the installer and
uninstaller, the Windows file icon (`assets/brand/clipcraft.ico`), the Discord application icon and
the small badge on Discord cards, and the top-left of the window beside the wordmark.

Files in `assets/brand/`: `mark-lime.svg`, `mark-black.svg` and `mark-white.svg` (the scalable
masters), `clipcraft-48.png`, `clipcraft-128.png` and `clipcraft-256.png`, and `clipcraft.ico`. Use
the SVG when you can; the PNG and ICO files are exports (see "Rebuild the kit").

| Background | Mark to use |
| --- | --- |
| Charcoal or the app's dark surfaces | lime |
| Paper or other light surfaces | charcoal (lime on paper is only a display accent, never for small sizes) |
| A photo or video frame | white, or charcoal on a light frame, with the clear space kept |

Do not: stretch, rotate, outline, add a shadow or gradient, recolour the mark in any other colour,
close its opening, crop it to a circle without the padding the square exports already include, put
it on a busy picture, or place text inside its opening. Do not use the mark to suggest another
company's endorsement, and never combine it with a publisher's logo.

## Colours in the app

The window is dark. Every colour comes from one list in `apps/clipcraft/src/kit.rs`; use those
names and do not write a colour into a screen.

| Name | Value | Used for |
| --- | --- | --- |
| `CANVAS` | `#111113` | the page behind everything |
| `SIDEBAR` | `#161619` | the left rail and the sidebars |
| `PANEL` | `#1B1B1F` | panels, dialogs and notification cards |
| `CARD` | `#202025` | cards and rows inside a panel |
| `CARD_HOVER` | `#29292F` | a card or row under the pointer, the chosen row |
| `BORDER` | `#2C2C32` | thin outlines and dividers |
| `CONTROL`, `CONTROL_HOVER`, `CONTROL_BORDER` | `#2E2E35`, `#3A3A42`, `#404049` | buttons and fields |
| `TEXT` | `#F0F0F4` | normal text |
| `MUTED` | `#9696A1` | help text, labels, secondary information |
| `FAINT` | `#5C5C66` | disabled text and the quietest hints |
| `LIME` | `#BEFF2B` | the one primary action, the live state, the chosen state |
| `DANGER` | `#FF7070` | errors and the delete button |
| `WARNING` | `#FFBE5C` | warnings |
| `INFO` | `#7AAAFF` | information |

Rules:

- **One lime thing at a time.** Lime marks the action that matters now (Start clipping, Save the
  last moments, Export) and the live or chosen state. Do not use it for decoration or for body text.
- **Red only for errors and deleting.** A warning is amber and information is blue; a message uses
  its kind's colour only in the badge, the glow and the time line (see
  [notifications.md](notifications.md)).
- **Contrast.** Normal text is `TEXT` or `MUTED` on the dark surfaces. `FAINT` is for disabled or
  decorative text only, never for something the person has to read. Meaning is never given by
  colour alone: a state also has a word, a glyph or a position.
- The mark's own signal lime (`#C2F84F`, above) and the app's `LIME` (`#BEFF2B`) are close but not
  the same value. Keep the mark's artwork exact, and use `LIME` for interface elements, until the
  two are deliberately made one.

## Type, spacing, corners and controls

Sizes, spacing and corners come from `kit::text_size`, `kit::space`, `kit::radius` and
`kit::control`; use them instead of numbers.

| Scale | Values (pixels) |
| --- | --- |
| Text | caption 12, small 13, body 14, lead 16, title 20; the page heading is larger |
| Spacing | 4, 8, 12, 16, 20, 24, 32 (`XS` to `HUGE`) |
| Corners | 6, 10, 14, and a pill of 17 |
| Control height | 28 compact, 34 standard |

Headings are SemiBold, buttons and labels Medium, text Regular. Labels use sentence case ("Start
clipping", "Save last 30 s"), not Title Case or capitals. A screen has one heading, rounded cards
for groups, thin dividers between rows, and room around things instead of boxes inside boxes.

## Icons

The interface icons are original artwork (`assets/icons/`, listed in `kit::Icon`), drawn from
scratch on a 24-unit square as line icons (a stroke of about 2 units, `currentColor`) with no fill
unless the shape is solid by nature (play, record). They take the colour of the text they sit
beside.

To add one:

1. Draw it on the 24-unit square in the same weight as the others; do not trace or copy an icon
   set, and do not take one from the web.
2. Add `assets/icons/<name>.svg`, its `<name>.svg.license.md` sidecar (copy an existing one) and a
   row in `ATTRIBUTION.md`.
3. Add it to `kit::Icon` (and its `ALL` list), then run `cargo xtask assets`, which fails when a
   sidecar or a row is missing.

## Games, colours and marks

Each game in the sidebar gets a coloured disc with a letter or a mark; the person can change both
(swatches, or any colour with the picker). Defaults come from a hash of the game's name, so a game keeps the same colour between runs and
between computers. ClipCraft ships no publisher logo, box art or screenshot. Where a game's
icon appears on a Discord card it is a fixed Discord-hosted reference that ClipCraft does not
download (see [discord-presence.md](discord-presence.md)).

## Writing

- Plain words for the person, not for the program: "Moved 3 clips to the Recycle Bin", not
  "Operation completed".
- Say what happened, then what to do. One or two sentences, no error codes, no blame.
- Name the thing: the clip's name, the folder, the setting.
- Do not say a thing is safe, fixed or verified unless it was checked; the gap report is the place
  for what was not.
- Short help beats a paragraph: a **?** button or one line under a setting.
- The wordmark and the product are "ClipCraft"; the mark is "CUT"; a saved recording is a "clip";
  the recording kept in memory is the "replay".

## Screenshots and sample media

Screenshots in the README and docs show real windows, taken on a private display (1440×900) with
original or generated sample footage; no game footage, publisher imagery or other people's
content, and no personal folders or names. Each screenshot has a `.license.md` sidecar saying
where it came from and a row in `ATTRIBUTION.md`; `cargo xtask assets` checks both. Reference
screenshots of other products stay on a local machine and are never committed or used as assets.

## Adding something visual: checklist

1. Use the existing colours, sizes, icons and the message design; add a new one only by changing
   `kit.rs` (or `notify.rs`) and this page together.
2. Run the app, look at the real result at a normal and a small window size, and keep before and
   after screenshots with the change.
3. Check that every control has a name a screen reader can say (the app's accessibility test does).
4. Add provenance (sidecar and `ATTRIBUTION.md` row) for any new file.

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
