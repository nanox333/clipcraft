# ADR 0002: Capture platform matrix and Windows-first path

- **Status:** Accepted as the M0 plan. Every row is **unmeasured**: it comes from public platform
  documentation, not from a benchmark on reference hardware. The M2/M3 spikes replace each
  "expected" with a measured result, or supersede this ADR.
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

## Context

Capture is the highest-risk subsystem (`PLAN.md`, "Capture and media design"). M0 requires a
capability matrix and a single supported Windows path before any UI work depends on capture. This
ADR records that matrix and its failure behavior. It does not claim performance.

## Decision

Target **Windows 10 (version 2004, build 19041) and Windows 11 first**. Older builds are
unsupported and the app says so at launch instead of failing during capture. The minimum build is
provisional until the audio spike confirms per-process loopback availability.

| Area | Windows (first release) | macOS (M6) | Linux (M6) |
| --- | --- | --- | --- |
| Display / window video | Windows Graphics Capture (primary); DXGI Desktop Duplication (fallback for monitors) | ScreenCaptureKit, macOS 12.3+ | xdg-desktop-portal ScreenCast + PipeWire |
| System / game audio | WASAPI loopback; per-process loopback where the build supports it | ScreenCaptureKit audio, macOS 13+ | PipeWire / PulseAudio monitor sources |
| Microphone | WASAPI capture | CoreAudio / AVFoundation | PipeWire / PulseAudio |
| Permission model | None for WGC; microphone privacy toggle | Screen Recording and Microphone prompts (TCC) | Portal consent dialog per session on Wayland |
| Hardware encode | Media Foundation hardware H.264 MFTs (vendor drivers expose NVENC, AMF, Quick Sync) | VideoToolbox | VA-API, vendor drivers |
| Known unsupported | Protected/DRM content (captured as black), exclusive-fullscreen apps that block WGC, elevated windows when ClipCraft is not elevated | Same DRM limits | Compositors without ScreenCast portal |

**Fallback behavior.** Each provider reports a typed capability set. When capture, audio, or a
hardware encoder is unavailable the UI shows the specific missing capability and keeps the library
and editor usable. No fallback injects into a game process, loads a driver, or bypasses DRM.

**Software encode.** [ADR 0004](0004-filmcraft-crates.md) identifies FilmCraft's pure-Rust H.264
encoder (`filmcraft-h264enc`) as the software fallback candidate. Its 1080p60 speed and quality are
unmeasured; confirming them is an M2 spike deliverable. Until it is wired in, export and capture
report "no encoder available" rather than producing a file.

## Consequences

- `clipcraft-platform` stays the only crate allowed platform `unsafe` (see `AGENTS.md`), behind a
  safe `Result` API.
- Release claims for any row need measured evidence in `docs/gaps.md`: 1080p60 capture, hotkey
  latency, dropped frames, A/V drift, and power draw on a named reference machine.
- macOS and Linux rows are design intent only until M6.
