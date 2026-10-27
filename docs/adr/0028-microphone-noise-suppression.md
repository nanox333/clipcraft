# ADR 0028: Offline microphone noise suppression

- **Status:** Accepted and implemented. Generated DSP/MP4/MKV tests, a Windows default-input
  adapter check, native settings review and `cargo xtask ci` pass (2026-10-27). Real speech
  listening quality and physical-device recovery remain open.
- **Date:** 2026-10-26

## Context

Microphone capture needs an optional way to reduce background noise such as fans and keyboard
noise while retaining speech. The feature must operate offline, keep bounded memory and work,
preserve the picture clock and existing mute/push-to-talk controls, and avoid a non-Rust runtime
or processing inside the device callback. It must not alter game/system sound.

## Dependency decision

Use [`nnnoiseless` 0.5.2](https://github.com/jneem/nnnoiseless/tree/v0.5.2), a Rust port of
RNNoise, with `version = "=0.5.2"` and `default-features = false`, resolved in `Cargo.lock`.
The reviewed release tag `v0.5.2` points to commit
`924a2dd143ccad7bce9e5bda061b60ca32911a67` (released 2025-12-18).
The release's built-in [`src/weights.rnn`](https://github.com/jneem/nnnoiseless/blob/v0.5.2/src/weights.rnn)
is 87,521 bytes. Use only that compiled-in model: no downloads, user-provided models, training,
model discovery or network requests are needed.

The crate and its embedded default model are distributed under the project's BSD-3-Clause
[`COPYING`](https://github.com/jneem/nnnoiseless/blob/v0.5.2/COPYING). Preserve its copyright
and full redistribution notice in
[`docs/licenses/nnnoiseless-BSD-3-Clause.txt`](../licenses/nnnoiseless-BSD-3-Clause.txt) and
record the shipped model in [`ATTRIBUTION.md`](../../ATTRIBUTION.md).

Disabling default features excludes the crate's command-line binary, `dasp` adapter, C API
and training tooling. The enabled direct dependencies are `easyfft` and `once_cell`.
`easyfft` wraps Rust `realfft`/`rustfft`; none of these requires a C/C++ denoiser, a bundled
executable or Python at build time or runtime. The reviewed dependency manifests use
permissive licenses; the exact resolved transitive graph remains subject to the normal
`cargo-deny` gate and lockfile review.

### Alternatives considered

- [`rnnoise-rs`](https://github.com/lovelace-labs/rnnoise-rs) 0.1.0 is a pure-Rust BSD-3-Clause
  port of the larger 2024 RNNoise model. It explicitly licenses its embedded weights and has
  a dependency-free default path. Its first release/repository is substantially newer, and
  the current-model path has a much larger model and higher processing cost than the older
  model in `nnnoiseless`. Its published performance figures are from different hardware and
  are not ClipCraft measurements. It remains a possible future quality/performance experiment.
- [`DeepFilterNet`](https://github.com/Rikorose/DeepFilterNet) has a Rust `libDF` inference path
  and MIT/Apache-2.0 code licensing. It requires a broader inference/model dependency review
  and integration than this first microphone control; its optional Python training tooling
  is unnecessary for Rust inference. It is not rejected merely because that tooling exists.
- [`sonora`](https://github.com/dignifiedquire/sonora) offers a pure-Rust BSD-3-Clause port
  of WebRTC processing, including Wiener noise suppression. Its wider echo-cancellation,
  gain-control and resampling pipeline is unnecessary for this initial neural suppression
  control and would need its own focused review.
- [`webrtc-audio-processing`](https://github.com/tonarino/webrtc-audio-processing) and
  [`rnnoise-c`](https://docs.rs/rnnoise-c/) are bindings to non-Rust processing libraries;
  they do not fit the product runtime constraint. The latter is deprecated in favor of
  `nnnoiseless`.

Reusing the reviewed Rust implementation avoids inventing a speech denoiser or presenting
a simple noise gate as equivalent to one. This choice does not establish Discord/Krisp
quality, keyboard-removal guarantees or any real-game performance claim.

## Processing contract

- The setting is **off by default** and belongs to microphone recording. Existing settings
  without the field retain their current behavior. The microphone test meter must describe
  processed sound. M3.1b replaces the initial raw meter with an explicit local listening test.
- Put the portable DSP adapter in `clipcraft-record`, after source-format validation and
  channel/sample-rate conversion, on the existing recorder worker. The OS callback continues
  to copy samples into its bounded queue only. Do not add a thread per frame.
- RNNoise consumes exactly **480 mono samples at 48,000 Hz** per call (10 ms). Keep input,
  output and partial-frame storage bounded; use fixed arrays at the dependency boundary.
  Stereo microphone channels use independent state; only an explicit mono choice folds them
  before suppression. Mono needs one model, stereo two. The requested clip channel layout is
  applied afterwards. Other supported source/clip rates need bounded streaming
  conversion to/from the 48 kHz processing rate; format support must be stated explicitly.
- The library uses `f32` values with signed 16-bit PCM amplitudes, rather than normalized
  floats. Replace non-finite input with silence, clamp normalized values, multiply by 32768,
  limit positive amplitude to 32767, then divide processed values by 32768 and sanitize/clamp
  again before mixing. Do not allow NaN/infinity to poison recurrent state or AAC input.
- Each microphone stream owns its denoiser and partial-frame state. Create fresh state on
  a new recording, device/format change or recovery discontinuity. Do not share state across
  microphones, system sound or recordings. Apply live mute/push-to-talk controls to the
  emitted samples so queued DSP history cannot bypass them.
- The analysis/synthesis pipeline introduces **480 samples (10 ms) of algorithmic delay**,
  and completing an input block can require up to another 10 ms. Account for both against
  the recorder's existing 40 ms arrival slack and retain the original sample timeline.
  Do not relabel delayed speech as current sound, discard initial speech or let delay grow
  with elapsed capture time. Warm up on the same long-lived worker so thread-local FFT
  planning and scratch allocation do not occur in a device callback.
- Capture is continuous; an empty source read is temporary unavailability, not end-of-file.
  Retain partial blocks across reads/ticks and do not zero-pad/flush them on each temporary
  drain. A replay save snapshots the continuous encoded buffer and does not reset DSP state.
  Any final boundary still follows the recorder's bounded end-of-clip policy, with no
  invented source samples or extra clip duration.
- Source loss, invalid formats and processing failures use the existing actionable audio
  error/recovery path and preserve video and saved clips. A failed enabled feature must not
  silently pretend that raw sound was suppressed.

## Safety boundary

ClipCraft adds no `unsafe` DSP or FFI. `nnnoiseless` exposes a safe API but assumes fixed
frame dimensions and contains assertions/indexing; use the fixed 480-sample arrays and the
compiled-in model exclusively. Its reviewed non-C-API `unsafe` is the immutable `u8` to `i8`
weight-slice view in `src/rnn.rs`: element sizes/alignment agree and all byte patterns are
valid. No model parser receives untrusted input from ClipCraft.

The FFT dependencies contain their own unsafe implementation details, including scratch-buffer
handling and architecture SIMD. They remain behind their safe, dimension-checked APIs; this
does not expand the first-party platform-unsafe exception. `easyfft` caches planning and
scratch storage per thread. Validate the resolved versions and their licenses with the lockfile
and quality gate, keep sizes constant, and do not make recursive/nested FFT calls on the worker.

## Verification and limits

Before marking the feature complete, require generated-noise attenuation, split-chunk/partial
frame equivalence, sample-count and impulse-timing checks, disabled bypass, mixed/separate track
isolation, rate/channel validation, non-finite samples, mute/push-to-talk and device recovery
tests. Measure processing time on the reference CPU in an optimized build, without equating a
synthetic microbenchmark with game FPS impact. Inspect the native Audio settings at a
representative desktop size and check keyboard operation. Run `cargo xtask ci` and preserve
real-device/privacy limits in [`docs/gaps.md`](../gaps.md).

Verification evidence and measured quality/performance limits are in [testing](../testing.md)
and [gaps](../gaps.md). The native 1440×900 build review and automated keyboard checks cover the
switch and mono guidance; M3.1b replaces the original raw meter with processed listening.
The five-second Windows default-input check saves no sound and proves finite sample delivery,
not listening quality. Loud full-band noise was reduced less than the quieter stationary fixture.
`recording.test_microphone` validates a typed plan shared by desktop/CLI. The native test runs
the same microphone factory and lane conversion/gain before output and metering, with one
worker, one replaceable settings slot and a 200 ms playback queue. Callback draining uses
`try_lock` and writes silence on underrun/contention; it allocates no buffers and performs no DSP.
Stop/leave immediately mutes output, with eventual same-thread cleanup of a pending OS open.
The sole-worker permit prevents repeated starts from accumulating blocked device opens. After
60 seconds the test closes itself. It is pre-AAC, local/unsaved, independent of recording/PTT
gates, and explicitly requires the user to start it; headphones prevent acoustic feedback.

Real microphone listening quality, echo cancellation, automatic gain control, suppression strength
presets and comparisons with Discord's proprietary processing remain outside this first slice.
