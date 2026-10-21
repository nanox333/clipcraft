# ADR 0014: Windows D3D11 capture-to-encode surfaces

- Status: implemented for Windows SDR; AMD generated-window evidence, remaining gates open.
- Date: 2026-10-21
- Work: M5.2, G0/G1; Windows SDR first, existing CPU/HDR path retained.

Windows Graphics Capture callbacks expose a borrowed D3D11 texture. Copy it on the GPU into a
bounded pool of owned BGRA textures before the callback returns. Never keep the capture frame-pool
texture for asynchronous use. Arc leases prevent recycling a surface while a recorder or screenshot
consumer owns it. The pool holds at most four textures and 128 MiB; outstanding old-size leases are
limited by the recorder's one-slot mailbox and single encoding worker. Oversized or unsupported
surfaces use CPU capture. The pixel budget must fit at least two surfaces: latest holds one lease,
so a one-slot pool would permanently repeat its first picture. Pool exhaustion drops an incoming
picture rather than growing memory.

Use the capture device with ID3D11Multithread protection. Media Foundation receives the same device
through IMFDXGIDeviceManager and MFT_MESSAGE_SET_D3D_MANAGER. A D3D11 video processor converts BGRA
to limited-range BT.709 NV12 on that device. IMFVideoSampleAllocatorEx owns a bounded NV12 sample
pool, so an encoder retaining an input sample cannot race with reuse of its texture. GPU copies
and conversion are permitted; no full-frame CPU Map/readback/re-upload is allowed on the native
path. CPU fallback and screenshot readback remain explicit and fallible. HDR, scaling/cropping and
unmatched surface/encoder combinations retain the existing tested CPU conversion route initially.

All new unsafe code stays in clipcraft-platform's d3d11.rs, mf_encoder.rs and mf_events.rs. Each block documents
the COM ownership, descriptor bounds, mapped-memory lifetime and immediate-context synchronization.
Never infer device compatibility from a non-null pointer: match the D3D device and validate format,
dimensions and driver API results. Balance Map/Unmap on every success path. Media Foundation APIs
remain dynamically loaded, preserving startup without the Media Feature Pack.

The asynchronous event callback resolves an `AgileReference<IMFMediaEventGenerator>` on its own
thread, completes `EndGetEvent` there and queues only validated integer event types. No raw COM
event reference crosses the thread boundary. A mutex/condition variable bounds delivery to 64
events; a weak owner reference breaks generator/callback ownership cycles. Callback errors stop
rearming and wake the encoder worker; input and drain stalls are bounded to two seconds.

Failure must preserve already saved clips and report a reason. Native setup may decline before
its first submitted frame; CPU NV12 input then continues. A running GPU/device failure is reported
through the recorder's existing bounded fallback/restart behavior. Restart clears the incompatible
encoded replay stream and begins a new software stream; unsaved pre-restart replay is lost, as
with the existing hardware failure policy. This remains a documented reliability limitation.
No injected hooks, drivers,
privileged helpers, hosted services or new non-Rust runtime are introduced.

Verification: deterministic lease/bounds/format tests; real generated-window WGC-to-MP4 tests;
decoded color/range and frame ordering; screenshot readback; CPU fallback; repeated start/stop;
resize/device-loss cases where practical; CPU/RSS/frame-time comparison with identical generated
workload; two-hour bounded replay/save soak. Real-game impact is separate from generated workload
measurements. Never claim an unrun GPU/vendor/format combination is supported.

Measured results and remaining gates: [Windows performance report](../windows-performance.md).

Public API specifications:

- https://learn.microsoft.com/en-us/windows/win32/medfound/mft-message-set-d3d-manager
- https://learn.microsoft.com/en-us/windows/win32/api/mfidl/nn-mfidl-imfvideosampleallocatorex
- https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11videocontext-videoprocessorblt
