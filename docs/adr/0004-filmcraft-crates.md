# ADR 0004: Consume FilmCraft crates as pinned git dependencies

- **Status:** Accepted. Supersedes the "FilmCraft reuse" row of ADR 0003 and corrects the software
  encoder paragraph of ADR 0002.
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

## Context

ADR 0003 deferred reuse until an API review. FilmCraft (`storytold/filmcraft`, MIT OR Apache-2.0,
pure Rust, clean-room) was reviewed at commit `6c3948a`. It already provides the pieces ClipCraft
planned to write: an exact tick clock, MP4/MOV demux and mux, a pure-Rust H.264 decoder and
encoder, frame and audio buffer types, and OS hardware codec adapters.

## Decision

1. **Dependency form.** Depend on FilmCraft crates through `git` plus a pinned `rev` in the
   workspace manifest. `deny.toml` allows only that one git source. Updating the pin is a reviewed
   change that runs the full gate.
2. **Clock.** ClipCraft adopts FilmCraft's `Tick` (254 016 000 000 per second, `i64`) and
   `FrameRate`. `clipcraft-time` re-exports them and adds validated constructors for untrusted
   input plus audio `SampleRate` math. The older nanosecond clock is gone. Every common frame
   rate (including 23.976, 29.97 and 59.94) and sample rate (8 kHz to 192 kHz) is an exact whole
   number of ticks, so no rounding is needed.
3. **Used now.** `filmcraft-time` and `filmcraft-isobmff`: `clipcraft-media` probes MP4 and MOV
   files with the ISO BMFF demuxer after its own header-size check. `filmcraft-codecs` and
   `filmcraft-media`: `clipcraft-player` opens clips through the streaming file reader and decodes
   video (H.264 and the other codecs that hub registers) on a worker thread.
   `filmcraft-h264enc` is a dev-dependency used to generate test clips without `ffmpeg`.
4. **Planned.** `filmcraft-h264enc` as the software encoder; `filmcraft-platform` for hardware
   codecs (see below); audio output from `filmcraft-codecs` audio decoding.
5. **Not used.** `filmcraft-export`, `filmcraft-render`, `filmcraft-project` and the editor crates.
   They pull in most of FilmCraft and ClipCraft has no project format (ADR 0001).
6. **Toolchain.** FilmCraft requires Rust 1.95, so the workspace `rust-version` is 1.95.

## Open items

- `filmcraft-platform` is the one FilmCraft crate that contains `unsafe`. ClipCraft's rule (`AGENTS.md`)
  keeps `unsafe` inside `clipcraft-platform`. Before depending on it, either record an ADR that
  accepts its `unsafe` as a reviewed third-party dependency or write ClipCraft's own adapters.
- The pure-Rust H.264 encoder's speed and quality for 1080p60 are unmeasured here. M2 must measure
  them on reference hardware before it is called the software fallback.
- Only MP4 and MOV are probed so far. FilmCraft has Matroska, WebM, AVI and MPEG-TS crates that
  can extend `clipcraft-media` the same way.

## Consequences

- Probe durations and frame rates are exact and comparable with FilmCraft projects.
- ClipCraft inherits FilmCraft's behavior and release cadence for these crates; the pin keeps
  builds reproducible and makes upgrades deliberate.
- Advisory auditing (`cargo deny check advisories`) still reports an unmaintained `ttf-parser`
  through egui; it is not part of the gate.
