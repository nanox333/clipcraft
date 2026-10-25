# ADR 0026 — Shared bounded GPU composition with CPU fallback

**Status:** accepted for M11.1i, 2026-10-25; [issue #12](https://github.com/nanox333/clipcraft/issues/12).

## Decision and provenance

Editor preview and sequence export call the same safe-Rust compositor, in `clipcraft-export`.
It aspect-fits without upscaling, applies the existing seven colour effects, composites over
opaque black, applies video fades, then draws ordered FilmCraft text masks with their opacity.
Portrait/ultrawide input is no longer independently squeezed along each axis to fit the canvas.
Preview uses the selected export canvas and integer-Tick source/timeline mapping. Font rasterization
and existing export YUV pre-reduction/RGBA conversion remain CPU work; no decoding algorithm changes.

Use exact `wgpu = 27.0.1` and `pollster = 0.4.0` (both MIT OR Apache-2.0). wgpu's reviewed
safe interface supplies device, queue, storage-buffer, compute and readback operations; its native
Vulkan/Metal/DX12 implementation contains dependency-internal platform unsafe. No first-party
unsafe or direct platform binding is added. wgpu was already present through eframe; enabling
native backends adds locked backend dependencies. The workspace license policy still applies. Naga's `hexf-parse 0.2.1` declares CC0-1.0 in its
published registry manifest (checksum `dfa686283ad6dd069f105e5ab091b04c62850d3e4cf5d67debad1933f55023df`).
It parses hexadecimal float literals for the embedded shader; it is Rust, with no unsafe blocks
or external executable. CC0 is a public-domain dedication with fallback permission, without a
patent grant. A package/version-specific cargo-deny allowance records this permissive reuse;
CC0 is not globally allowed and no copyleft exception or project-license change is introduced.

All host/UI/tool implementation remains Rust. `compositor/kernel.wgsl` is original fixed GPU
instruction source embedded in the Rust binary, analogous to shader instructions required by the
native UI. It is not a foreign-language application, sidecar or required external compiler/build
step; wgpu compiles it in-process. This is the interpretation of the Rust-only rule for this
explicitly authorized GPU task. No runtime shader/plugin input is accepted. No Concat code,
shader, assets or FFmpeg binding is copied or translated; Concat informed the performance question.
The project license is unchanged.

## Selection, failure and concurrency

Opaque matching-size frames with no edits return their owned pixels directly. Other frames lazily
try one shared hardware context. CPU/Other adapters are rejected automatically. An eight-effect
startup probe must match the CPU within one 8-bit channel level before a hardware context is used.
Only one GPU frame submits at once; a contending worker immediately uses CPU. The mutex/context
owns reusable buffers, not a job queue. Driver initialization happens on workers, never the UI;
OnceLock initialization can make another worker wait for first initialization.

Missing/unsupported adapters, validation/submission/readback errors, device loss and caught Rust
panics retire the context and complete the frame on CPU. It is not retried until restart. Poll and
map waits are each limited to 500 ms; native driver initialization/compilation/submission/drop are
not preemptible, so this is no hard end-to-end deadline. Extreme downscales exceeding 4,096 source
samples per output invocation also retire this optional context and use CPU. No exported media
or source file is exposed to the GPU code. Export's existing collision-safe publication and
cancellation checks before/after composition remain in force.

Preview snapshots its immutable recipe under a short mutex and renders on its existing decode
worker, within foreground decode admission. Changes invalidate the pending picture through the
existing player seek revision; old completions/errors cannot replace a newer recipe. Prepared
sources start with their actual recipe. Adjacent plain cuts need no visual refresh. The UI still
paints text selection/drag handles but does not paint baked fades or glyphs twice. Render failures
are actionable and edits can reopen the bounded preview. No new thread, queue, engine operation,
command schema, project format or settings switch is introduced.

## Budgets and evidence limits

Source: exact RGBA length, nonzero, <=8192 per edge and <=33,177,600 pixels. Canvas: 2–3840 per
edge and <=8,294,400 pixels. At most 256 overlays, each <=2,097,152 pixels, <=8192 per edge;
combined masks <=16 MiB. Signed positions stay within ±8192, opacity/fades are finite 0–1.
Validate before allocation/submission. Text caches retain <=16 MiB each. Reusable GPU buffers
retain their high-water capacities until context release/restart: at most about 206 MiB of managed
buffers (source+axes, output, staging, masks, uniforms). CPU inputs, host upload copies, decoded
frames, UI textures and driver/codec storage are additional; this is not a total RSS/VRAM cap.

Every frame still uploads CPU RGBA and reads pixels back before UI upload or encoding. This is
not zero-copy rendering or a hardware performance promise. `Compositor::software_gpu` is a strict
explicit diagnostic path: no hidden CPU substitute if the required adapter fails. The verification
example checks 96 real software-Vulkan cases including alpha, framing, all effects, fades and
ordered/clipped text; normal CI checks CPU/reference/preview parity and adapter retirement when
available. Physical GPUs, Metal/DX12 runtime, power and end-to-end preview/export performance need
separate measurements. See [testing](../testing.md) and [performance](../performance.md).
