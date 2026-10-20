# ClipCraft roadmap

**Stage:** early build · **Target:** offline-first Windows desktop capture and editing

ClipCraft is a Rust-only native desktop product. Every core workflow must run offline on the user's
computer. No hosted backend, account, upload, sync, share-link service, or server process is planned.
The complete feature proposal and acceptance details are in [`PLAN.md`](PLAN.md).

## Release gates

- **Prototype:** clean Cargo workspace; synthetic media can flow through the exact-time model,
  local library index, cut/trim command, and standard video export.
- **Local alpha:** Windows capture, bounded replay buffer, hotkey, synchronized audio, local library,
  basic cut/trim editor, and MP4 export pass end-to-end checks without a network connection.
- **Beta:** crash recovery, storage limits, device-loss and full-disk recovery, signed installer,
  accessibility review, measured performance budget, and two-hour capture soak pass on reference
  hardware.
- **Additional platforms:** macOS and Linux providers pass the same local capture, audio, recovery,
  and export contract.

## Milestones

| ID | Status | Outcome |
| --- | --- | --- |
| M0 | 🟡 Software exit criteria met; hardware evidence open | Rust workspace, layering rules, CLI, CI gate, and the synthetic video/audio timestamp check are done; decisions are recorded in ADRs 0002–0003. Capture APIs are not yet benchmarked on reference hardware, so M0 is not marked complete |
| M1 | 🟡 Software done; Windows/macOS evidence open | Exact time primitives, audio sample math, bounded queues, a persistent rebuildable library index, MP4/MOV/MKV/WebM probing, in-app playback with sound and full controls, cached thumbnails, favorites/collections/games, and trim-and-export (no re-encode, frame-accurate start, MP4 output, progress and cancel; ADR 0010) are in place. Not done: AVI/WMV probing, playback measurements on reference hardware |
| M2 | 🟡 Pipeline proven on Linux; Windows capture unrun | Capture model, replay buffer, H.264 encode, and MP4/MKV save are built and tested end to end against an X11 screen (see [ADR 0006](docs/adr/0006-m2-capture-and-replay-buffer.md)); the app records, saves a clip from a global hotkey, and honours the quality, framing, folder, and storage-limit settings. The Windows Graphics Capture provider compiles but has not run on Windows hardware, so M2 is not marked complete |
| M3 | 🟡 In progress | Microphone and system audio as AAC tracks, mixed or separate, mute/level/push-to-talk, sync against the picture clock, device-loss recovery: done and tested with generated sources; Linux system sound (PulseAudio/PipeWire) and post-roll are done too; real-device runs and macOS system sound are open |
| M4 | 🟡 Partial | The clip-first library is in place: search, sorting, persistence, thumbnails, favorites, collections, and games. A basic trim is built (see M1); a re-encoding editor, resize and format choices are not |
| M5 | 🟡 Software done; real-hardware evidence open | Crash and full-disk safety, a soak tool with resource budgets, simulated suspend/resume (stale buffer dropped), keyboard focus ring and named controls for screen readers, `clipcraft-cli diagnose` and a Copy report button, and a Linux release package are built and tested; see the [release checklist](docs/release-checklist.md) for what is still open (signed Windows/macOS installers, two-hour run, real screen reader, real devices) |
| M6 | 🟡 Linux started | Linux: X11 capture, VA-API encoding, PulseAudio/PipeWire sound, and Wayland capture through the portal and PipeWire (ADR 0012; pieces tested, no real desktop chooser seen yet). macOS: not started |

Issue-level tasks require a problem statement, explicit limitations, measurable acceptance criteria,
and test evidence. See the [`initial backlog`](docs/backlog.md) and
[`issue requirements`](docs/issue-requirements.md). No milestone can be marked complete from a
successful build alone; it needs end-to-end evidence for its user workflow.
