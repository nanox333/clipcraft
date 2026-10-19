# ADR 0005: MPL-2.0 exception for the symphonia audio decoders

- **Status:** Accepted, reversible. Needs maintainer confirmation because it relaxes a rule in
  `AGENTS.md` ("permissively licensed dependencies").
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

## Context

`filmcraft-codecs`, the container and codec hub behind clip playback (ADR 0004), depends on
`symphonia` for MP3, FLAC, Vorbis, ALAC and some AAC/MP4/Ogg/RIFF audio decoding. All thirteen
`symphonia-*` crates are licensed MPL-2.0, a file-level copyleft licence. Every other dependency in
the tree passes the existing allowlist. FilmCraft's own rules allow "MPL-2.0 used unmodified".

## Decision

Allow MPL-2.0 **only** for the thirteen named `symphonia*` crates, through per-crate
`[licenses.exceptions]` entries in `deny.toml`. The global allowlist is unchanged, so any other
MPL-2.0 (or copyleft) dependency still fails the gate. The crates are used unmodified from
crates.io; ClipCraft does not vendor, patch, or fork them. MPL-2.0 obliges anyone who modifies those
files to share the modified files, and requires licence notices when distributing them; neither
applies to unmodified use, but release packaging must carry the notices.

## Alternatives

- **Depend on narrower FilmCraft crates** (`filmcraft-isobmff`, `filmcraft-h264`) and write
  GOP-aware seeking, demux glue and audio decoding in ClipCraft. This avoids MPL-2.0 but duplicates
  the part of FilmCraft that already works, and loses Matroska/WebM/AVI playback.
- **Ask FilmCraft to put `symphonia` behind a Cargo feature**, then build ClipCraft without it. This
  is the cleanest long-term answer and would also drop MP3/FLAC/Vorbis support until needed.

## Consequences

- Reversing this ADR means deleting the exceptions and choosing one of the alternatives.
- The installer must include third-party licence notices, including MPL-2.0's, when it exists (M5).
