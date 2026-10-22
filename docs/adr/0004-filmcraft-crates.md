# ADR 0004: Consume FilmCraft crates as pinned git dependencies

- **Status:** Accepted. Supersedes the "FilmCraft reuse" row of ADR 0003 and corrects the software
  encoder paragraph of ADR 0002.
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

## Context

ADR 0003 deferred reuse until an API review. FilmCraft (`storytold/filmcraft`, MIT OR Apache-2.0)
was reviewed at commit `6c3948a`. It provides an exact tick clock, MP4/MOV demux and mux,
pure-Rust codec implementations, frame and audio abstractions, and OS hardware codec adapters.
ClipCraft consumes selected crates at a pinned revision. Because it reuses open-source code, this is
an independent product implementation, not a formal clean-room implementation.

## Decision

1. **Dependency form.** Depend on FilmCraft crates through `git` plus a pinned `rev` in the
   workspace manifest. `deny.toml` allows only that one git source. Updating the pin is a reviewed
   change that runs the full gate.
2. **Clock.** ClipCraft adopts FilmCraft's `Tick` (254 016 000 000 per second, `i64`) and
   `FrameRate`. `clipcraft-time` re-exports them and adds validated constructors for untrusted
   input plus audio `SampleRate` math. The older nanosecond clock is gone. Every common frame
   rate (including 23.976, 29.97 and 59.94) and sample rate (8 kHz to 192 kHz) is an exact whole
   number of ticks, so no rounding is needed.
3. **Used now.** `filmcraft-time` supplies exact `Tick` and `FrameRate` values to
   `clipcraft-time`. `filmcraft-isobmff` and `filmcraft-matroska` back probing, playback, recording,
   and trim export. `filmcraft-codecs` and `filmcraft-media` provide the playback decoder hub.
   `filmcraft-h264enc` and `filmcraft-aac` are used by the local recorder; `filmcraft-h264` and
   `filmcraft-platform` provide hardware encoder paths behind `clipcraft-platform`. ClipCraft owns
   its command engine, UI, capture model/providers, replay buffer, recording flow, player controls,
   index, and no-reencode trim behavior.
4. **Not used.** `filmcraft-render`, `filmcraft-project`, and FilmCraft's full editor workflow are
   not part of ClipCraft. The app has no user-facing project format or multitrack timeline (ADR 0001).
5. **Toolchain.** FilmCraft requires Rust 1.95, so the workspace `rust-version` is 1.95.

## Safety and open items

- FilmCraft's `filmcraft-platform` contains audited third-party `unsafe`; it is isolated behind
  ClipCraft's `clipcraft-platform` API. ClipCraft's own `unsafe` remains confined to its reviewed
  platform crate with `// SAFETY:` comments and tests. License and source pin updates require review.
- The pure-Rust H.264 encoder is wired as fallback but is too slow for reliable 1080p60 in the
  measured container; reference gaming-hardware performance is still open.
- Probing supports MP4/MOV and Matroska/WebM metadata. AVI/WMV are not probed or trim-exported.
- Real NVENC, VA-API, Media Foundation, and VideoToolbox hardware runs remain unverified.

## Consequences

- Probe durations and frame rates are exact and comparable with FilmCraft projects.
- ClipCraft inherits FilmCraft's behavior and release cadence for these crates; the pin keeps
  builds reproducible and makes upgrades deliberate.
- Advisory auditing (`cargo deny check advisories`) still reports an unmaintained `ttf-parser`
  through egui; it is not part of the gate.

M11.1a follow-up: [ADR 0017](0017-small-filmcraft-sequence-editor.md) adds pinned `filmcraft-edit`
and transient `filmcraft-project` track types for the small editor, plus AAC encoding in sequence
export. The full FilmCraft application, renderer and project-file workflow remain unused.
