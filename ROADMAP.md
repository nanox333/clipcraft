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
| M1 | 🟡 In progress | Exact time primitives, audio sample math, bounded queues, and a persistent rebuildable library index are in place; MP4/MOV file probing, in-app playback with sound and full controls (scrub, frame step, speed, loop, volume, tracks, fullscreen), and cached thumbnails are in place via FilmCraft crates; other-container probing and export remain |
| M2 | ⬜ Planned | Windows display/window capture and bounded replay buffer |
| M3 | ⬜ Planned | System/game audio, microphone, synchronization, post-roll, device recovery |
| M4 | 🟡 Partial | Local folder scanning, clip-first library shell, search, sorting, and path copy are in place; thumbnails, persistence, quick editing, and export remain |
| M5 | ⬜ Planned | Crash recovery, storage quota, installer, accessibility, performance hardening |
| M6 | ⬜ Planned | macOS and Linux local capture/audio providers |

Issue-level tasks require a problem statement, explicit limitations, measurable acceptance criteria,
and test evidence. See the [`initial backlog`](docs/backlog.md) and
[`issue requirements`](docs/issue-requirements.md). No milestone can be marked complete from a
successful build alone; it needs end-to-end evidence for its user workflow.
