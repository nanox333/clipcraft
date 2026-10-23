# ADR 0019 — Shared editor rendering and file-size presets

**Status:** accepted · **Date:** 2026-10-22 · **Scope:** M11.1b

[ADR 0020](0020-internal-editor-recovery.md) subsequently supersedes the session-only lifetime
with internal recovery. The rendering/export decisions and remaining media limits below continue
to apply.

## Decision

The user requested fixes for GIF timing and editor preview/export consistency, followed by the
existing file-size presets in the editor. Preserve the Rust-only, offline workflow, ordinary MP4
output and in-memory session model from ADR 0017. No new dependency, platform FFI boundary,
server, upload or project format is introduced.

The current editor includes independent text layers, seven colour effects and black/white fade
dips. This supersedes ADR 0017's deferral of those bounded operations. There is still one gapless
video sequence with linked source audio; detached audio, overlapping video and cross-dissolves
remain outside the slice.

Preview and export share one FilmCraft Inter SemiBold text raster implementation, including
wrapping, shadow and placement geometry. Rasterize at canonical export-canvas dimensions and
scale the pixels for preview, rather than laying out text separately in egui. Keep timing and
position separate from raster content so moving/fading a layer can reuse its pixels. Bound each
raster to 2,097,152 pixels, cap text layout height to the canvas and use separate 16 MiB
preview/export caches. Render active layers instead of keeping every layer mask. Preview sound
applies the same segment fade envelope as exported sound without changing the stored volume.

GIF frame delays use cumulative integer centisecond boundaries, including the final partial
frame. Preserve the rounded total duration instead of accumulating a rounded-down delay per
frame. Reserve at least two centiseconds for a very short final frame by borrowing from its
predecessor, avoiding reader-dependent handling of zero/one-centisecond delays.

## Sequence export

`SequenceRequest.preset` is optional and defaults to `None`, retaining the standard up-to-720p,
30 fps H.264 export with a 4 Mbps video target. Existing request JSON without the field is valid.
`Small`, `Medium` and `Custom` reuse the estimated 10 MB, 25 MB and 1–500 MB target model.

Reserve the continuous 160 kbps AAC stream and padding, then container/index overhead and bitrate
headroom before choosing the video bitrate. Cap video at 4 Mbps and reduce the canvas through
720p, 540p, 480p and 360p tiers as needed, without upscaling source pictures. Reject budgets that
cannot fit audio plus the minimum video bitrate. Size targets remain estimates; report actual
bytes and whether the target was met rather than promising a third-party upload limit.

The engine supplies the registered GPU encoder when the recording encoder preference permits it.
A device, encoded-stream or packet-accounting failure discards only the job's partial output and
restarts the sequence in software. Cancellation and filesystem failures propagate without a
fallback retry. Sequence decoding stays on its software path; hardware decoding is not claimed.
Keep exclusive temporary creation and no-replace publication, preserving sources and pre-existing
files on every failure path.

The UI opens an Export video window with standard, small, medium and custom choices. The shared
`editor.export` command returns `SequenceExported`: clip facts, optional `target_bytes`,
`within_target`, dimensions, encoder name and `hardware_decode`. The Rust result owns a `clip`
field, flattened during JSON serialization. The CLI therefore preserves the earlier top-level
`path`, duration, counts and byte fields, and adds export metadata alongside them.

## Validation and remaining limits

Require GIF duration regressions at 12/13 fps and fractional ranges, shared text-pixel and audio
fade checks, budget/dimension validation, generated multi-source preset exports and decoder checks,
hardware-fallback stand-ins, cancellation/collision preservation, UI screenshots/keyboard checks,
`cargo xtask ci` and documentation checks. Record actual evidence in [testing.md](../testing.md).
Real-GPU speed/quality, physical audio output and broader platform UI behavior remain separate
verification work. Normal codec/FPS, source-format, 30-minute sequence and session-lifetime limits
remain unchanged; see [editor.md](../editor.md).

M11.1d supersedes the fixed 720p/30 export limit with bounded resolution/FPS choices; see
[ADR 0021](0021-editor-export-quality.md).
