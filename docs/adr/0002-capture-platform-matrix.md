# ADR 0002: Capture platform matrix and Windows-first path

- **Status:** Accepted as the Windows-first target matrix. Implementation evidence has been added
  below; real reference-gaming-hardware results remain open. See [`docs/gaps.md`](../gaps.md) for
  the current evidence and known limits.
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

**Software encode.** FilmCraft's pure-Rust H.264 encoder (`filmcraft-h264enc`) is wired as the
recording fallback. In this shared 4-core container it cannot sustain 1080p60 and drops 73% of
pictures under the measured workload; 720p60 also drops about a third. Performance on a real gaming
PC remains unmeasured. Platform hardware encoder adapters are wired, but no real GPU encoder has run
in this environment. See [`docs/gaps.md`](../gaps.md).

## Consequences

- `clipcraft-platform` stays the only crate allowed platform `unsafe` (see `AGENTS.md`), behind a
  safe `Result` API.
- Release claims for any row need measured evidence in `docs/gaps.md`: 1080p60 capture, hotkey
  latency, dropped frames, A/V drift, and power draw on a named reference machine.
- Linux X11 capture, replay, system sound, and trim recording have end-to-end evidence in a virtual
  display. Wayland portal/PipeWire pieces are partly verified, but no real desktop chooser has
  delivered a frame. Windows capture compiles but has not run on Windows hardware; macOS capture is
  not implemented. These limits do not change the Windows-first release target.

## Implementation evidence update

The original table remains the target capability matrix. Current evidence is not equivalent across
platforms:

- **Linux/X11:** screen and single-window capture, global hotkey, bounded replay, MP4/MKV writing,
  framing, generated audio sync, and trim export have automated or virtual-display end-to-end tests.
- **Linux/Wayland:** the portal conversation and PipeWire receiver have component tests, but no real
  portal chooser has produced a captured frame. See [ADR 0012](0012-wayland-capture.md).
- **Windows:** Graphics Capture, global hotkeys, and Media Foundation code compile; none has run on
  a Windows machine. Protected windows, exclusive fullscreen, and hardware encoders remain open.
- **macOS:** no capture/audio provider has been implemented.
- **Real devices:** microphone/speaker capture and NVENC, VA-API, VideoToolbox, and Media Foundation
  hardware encoding have not been exercised on real hardware here.
