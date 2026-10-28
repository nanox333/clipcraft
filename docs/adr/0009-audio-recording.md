# ADR 0009: Recording sound

- **Status:** Accepted. End-to-end evidence uses generated sources. M3.1a adds a short Windows
  default-microphone adapter/denoiser check; real spoken clips, listening and physical-device
  recovery remain open. See [ADR 0030](0030-microphone-noise-suppression.md).
- **Date:** 2026-10-20

## Context

Clips were silent. The settings for the microphone, game sound, separate tracks, push-to-talk and
mono were saved but did nothing. Sound must stay in step with the picture for hours, survive a
device being unplugged, and never grow a queue without bound.

## Decision

- **One clock.** The picture's times come from a wall clock started with the recording. The
  `AudioEngine` is told how much time has passed and produces exactly that much sound per track
  (less 40 ms for sound still arriving). A source that is late is padded with silence; one that is
  ahead by more than 120 ms loses its oldest samples. Device clocks that run fast or slow therefore
  cannot drift away from the picture; the status shows the silence and the dropped milliseconds.
- **Lanes.** Each source is converted to the clip's channel count and sample rate (linear
  interpolation) in a lane; garbage samples (NaN, infinities) become silence. Sources assigned to
  the same track are summed and clamped.
- **AAC from FilmCraft** (`filmcraft-aac`, pure Rust) at the chosen rate and bitrate, one encoder
  per track. Units are buffered next to the video in a bounded `AudioBuffer`.
- **Layering.** `clipcraft-record` (L3) defines `AudioSource` and `AudioProvider` and knows no
  operating system. `clipcraft-audio-out` (L5) implements them with cpal: a thread per device owns
  the stream (streams are not always `Send`) and the device callback appends to a bounded queue.
  The engine builds the setup from the settings and exposes mute, push-to-talk and levels.
- **Files.** MP4 audio tracks carry an edit that skips AAC's warm-up unit and the part before the
  first picture; the file is interleaved. Matroska stamps every block, so the warm-up is left out.
  The picture is cut where the sound ends so both are the same length.
- **Failure.** A source that cannot be opened is skipped with a reason shown in the top bar; the
  video records regardless. A lost device goes silent, is reopened every two seconds, and rejoins
  when it comes back with the same format.

## Consequences

- System sound is a loopback of an output device on Windows, and on Linux the monitor of the
  default output through libpulse (loaded at run time in `clipcraft-platform::pulse`, the only
  sound `unsafe`; ALSA "Monitor of ..." inputs remain the fallback). macOS has no loopback and says
  so. The Linux path follows the default output rather than the chosen one.
- Linear resampling is adequate for game audio and not for music mastering.
- Real hardware remains the open evidence: first checks are the Settings microphone test meter,
  then a clip with a spoken word and a clap against the screen.
