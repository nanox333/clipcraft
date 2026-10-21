# ClipCraft roadmap

**Stage:** active prototype · **Last reviewed:** 2026-10-21 · **Target:** offline-first, game-aware capture and local editing

ClipCraft is a pure-Rust, offline-first native desktop product. Capture, library, editing, and export
must work offline. Optional user-configured client-side-encrypted backup is planned, with no
ClipCraft-hosted storage or account. The complete direction and constraints are in
[`PLAN.md`](PLAN.md).

## Release gates

- **Prototype:** local capture/replay/save, playback, persistent clip library, and no-reencode trim
  export are implemented. Linux/X11 has an end-to-end synthetic workflow; Windows capture runtime
  and real-device evidence remain open.
- **Local alpha:** Windows capture, bounded replay buffer, hotkey, synchronized audio, local library,
  basic cut/trim editor, and MP4 export pass end-to-end checks without a network connection.
- **Beta:** crash recovery, storage limits, device-loss and full-disk recovery, signed installer,
  accessibility review, measured performance budget, and two-hour capture soak pass on reference
  hardware.
- **Additional platforms:** macOS and Linux providers pass the same local capture, audio, recovery,
  and export contract.

## Milestones

Performance work M2.P1 has synthetic CPU measurements for shared replay snapshots, GOP boundary
caching and redundant pixel-write removal. [The review](docs/performance.md) records the results
and priorities across the workspace. Game frame-time comparisons, GPU readback removal and the
reference-hardware capture/soak gates remain open; no milestone status changes follow from this work.

| ID | Status | Outcome |
| --- | --- | --- |
| M0 | 🟡 Software exit criteria met; hardware evidence open | Rust workspace, layering rules, CLI, CI gate, and the synthetic video/audio timestamp check are done; decisions are recorded in ADRs 0002–0003. Capture APIs are not yet benchmarked on reference hardware, so M0 is not marked complete |
| M1 | 🟡 Software done; Windows/macOS evidence open | Exact time primitives, audio sample math, bounded queues, a persistent rebuildable library index, MP4/MOV/MKV/WebM probing, in-app playback with sound and full controls, cached thumbnails, favorites/collections/games, and trim-and-export (no re-encode, frame-accurate start, MP4 output, progress and cancel; ADR 0010) are in place. Not done: AVI/WMV probing, playback measurements on reference hardware |
| M2 | 🟡 Pipeline proven on Linux; Windows capture unrun | Capture model, replay buffer, H.264 encode, and MP4/MKV save are built and tested end to end against an X11 screen (see [ADR 0006](docs/adr/0006-m2-capture-and-replay-buffer.md)); the app records, saves a clip from a global hotkey, and honours the quality, framing, folder, and storage-limit settings. The Windows Graphics Capture provider compiles but has not run on Windows hardware, so M2 is not marked complete |
| M3 | 🟡 In progress | Microphone and system audio as AAC tracks, mixed or separate, mute/level/push-to-talk, sync against the picture clock, device-loss recovery: done and tested with generated sources; Linux system sound (PulseAudio/PipeWire) and post-roll are done too; real-device runs and macOS system sound are open |
| M4 | 🟡 Partial | The clip-first library has search, sorting, persistence, thumbnails, favorites, collections, and game labels. Original-quality trim and local H.264 compression with estimated 10 MB, 25 MB, and custom targets are built; generated MP4/MKV tests cover decoding, audio sync, cancellation, and file preservation. Recursive scans, automatic game tagging, broader export formats, and reference-machine export measurements remain open |
| M5 | 🟡 Software partial; hardware evidence open | Crash and full-disk safety, a soak tool with resource budgets, simulated suspend/resume (stale buffer dropped), keyboard focus ring and named controls for screen readers, `clipcraft-cli diagnose` and a Copy report button, and a Linux release package are built and tested. The optional native-surface handoff and CPU fallback are tested with stand-ins; no platform backend uses them yet, so a low-copy GPU capture-to-encode path remains planned in M5.2. See the [release checklist](docs/release-checklist.md) for open work |
| M6 | 🟡 Linux started | Linux: X11 capture, VA-API encoding, PulseAudio/PipeWire sound, and Wayland capture through the portal and PipeWire (ADR 0012; pieces tested, no real desktop chooser seen yet). macOS: not started |
| M7 | 🟡 Game-only arming implemented; platform evidence open | Vice-inspired matching, a user-managed allowlist, process observation, and automatic replay arming are implemented. X11 has synthetic window-to-process coverage; Windows runtime, real-game behavior, Wayland matching, and library auto-tagging remain open |
| M8 | ◷ Planned | Establish tested Windows/Linux/macOS desktop support through the shared Rust platform layer; run an Android screen-capture feasibility study without promising mobile support |
| M9 | ◷ Planned | Design and implement opt-in client-side-encrypted backup to a user-selected provider, including recovery and restore; all local workflows remain offline |
| M10 | ◷ Planned | Add controller shortcuts and optional Discord Rich Presence; prototype declarative themes, keyboard/mouse overlays, and a permission-limited plugin/shader extension model |
| M11 | ◷ Planned | Evaluate a focused one-track editor expansion and a local CLI/MCP adapter for footage management; no full NLE, AI service, or community feature is implied |

Issue-level tasks require a problem statement, explicit limitations, measurable acceptance criteria,
and test evidence. See the [`backlog`](docs/backlog.md) and
[`issue requirements`](docs/issue-requirements.md). No milestone can be marked complete from a
successful build alone; it needs end-to-end evidence for its user workflow.

Documentation ships with implementation: update `README.md` and `docs/gaps.md` in the same pull
request as every code change, then update milestone/backlog status and supporting architecture,
testing, CLI, release, or ADR guidance as needed. Pull-request CI enforces the README and gap-report
updates.

M7–M11 are proposals, not implemented capabilities. Each feature stays unstarted until its backlog
item's limitations, privacy model, platform matrix, and verification criteria are accepted and
evidenced. Android remains research-only, plugins require a reviewed isolation model, and backup
must encrypt before leaving the device.
