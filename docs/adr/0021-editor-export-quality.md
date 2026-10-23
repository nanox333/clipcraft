# ADR 0021 — Bounded editor export quality

**Status:** accepted · **Date:** 2026-10-23 · **Scope:** M11.1d, [issue #2](https://github.com/nanox333/clipcraft/issues/2)

The user approved 1080p/source-resolution editor output, 60 FPS, and explicit file-size reductions.
Add an optional `quality` object to `editor.export` and internal recovery, defaulting to
`{"resolution":"720p","fps":30}`. Valid resolutions are `720p`, `1080p`, and `source`; rates
are exactly 30 or 60. Unknown resolutions, unsupported rates and malformed values fail before
opening sources or creating output. Old request and recovery payloads retain their defaults.
This supersedes the fixed 720p/30 constraints in ADRs 0017 and 0019, without adding project files.

720p/1080p retain the existing first-source-height, 16:9 canvas and even rounding. Source uses
the first clip's native aspect ratio, fitted within 3840×2160 for landscape or 2160×3840 for
portrait. Output has at most 8,294,400 pixels. Smaller sources are not enlarged; mixed sources fit
with black bars. Video and text preview use the same canvas as export. Output rates sample
source motion on the selected integer frame clock; there is no interpolation. Existing editor
cuts/stepping remain on the 30 FPS editing grid. Output is bounded to 108,000 frames/30 minutes.

Standard video targets scale with pixels and FPS from the legacy 4 Mbps baseline, capped at
50 Mbps. File-size budgets reserve continuous 160 kbps stereo AAC and mux overhead, then cap the
video target to the standard target. Resolution tiers account for FPS; reducing pixels does not
silently change the selected rate. The dialog shows actual dimensions/FPS/target bitrate and
explains bitrate or resolution reductions. Reports add `fps`, target `video_kbps` and
`resolution_limited`; size/bitrate targets are estimates, not measured quality or service limits.

The shared settings helper drives software encoding, hardware requests, mux timestamps and the UI.
Text measures content height rather than allocating an empty full-canvas paragraph; shared
rasters support the bounded 4K canvas while retaining the 2,097,152-pixel per-layer and
16 MiB active-cache budgets. Oversized text reports an actionable reduce/shorten error.
Hardware failure retries software with the same resolved settings. Final partial video samples
are shortened; continuous audio follows exact cut positions without accumulating per-cut delay.
Sources and completed outputs remain unchanged on invalid input, cancellation or failure.
Recovery preserves quality choices without changing the version-1 ownership marker; absent fields
default, while invalid supplied values preserve the existing record and report an error.

No codec, capture-provider, required service, dependency, quick-trim behavior or GPU-performance
claim is added. The existing SDR H.264/AAC input, first-audio-track, output publication and resource
limits remain. Codec buffers are additional to the output-frame budget. Verification uses generated
Rust media, hardware stand-ins, external optional decoding oracles, and Linux virtual-display UI;
Windows/macOS hardware throughput and physical audio remain separate gates.
