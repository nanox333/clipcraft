# ClipCraft roadmap

**Stage:** active prototype · **Last reviewed:** 2026-10-25 · **Target:** offline-first, game-aware capture and local editing

ClipCraft is a pure-Rust, offline-first native desktop product. Capture, library, editing, and export
must work offline. Optional user-configured client-side-encrypted backup is planned, with no
ClipCraft-hosted storage or account. The complete direction and constraints are in
[`PLAN.md`](PLAN.md).

## Release gates

- **Prototype:** local capture/replay/save, playback, persistent clip library, and no-reencode trim
  export are implemented. Linux/X11 and Windows/AMD have generated capture-to-file evidence;
  Windows hotkey, real-game and broader real-device evidence remain open.
  M5.3 verifies a connected Linux/X11 generated-game → F8 replay → library/audio playback → cuts/text
  → export → normal/crash recovery workflow, with portable regressions and a network-call audit.
  This does not close the Windows or network-disabled release gates.
- **Local alpha:** Windows capture, bounded replay buffer, hotkey, synchronized audio, local library,
  basic cut/trim editor, and MP4 export pass end-to-end checks without a network connection.
- **Beta:** crash recovery, storage limits, device-loss and full-disk recovery, signed installer,
  accessibility review, measured performance budget, and two-hour capture soak pass on reference
  hardware.
- **Additional platforms:** macOS and Linux providers pass the same local capture, audio, recovery,
  and export contract.

## Milestones

M10.2b establishes the maintainer-selected A / CUT identity, with shared native/Discord product
artwork and a reproducible brand kit. M10.2c replaces three typographic game placeholders with
Discord-hosted icon references; no publisher image files or runtime lookup service are bundled.
Discord upload and profile rendering are separate verification steps.

Performance work M2.P1 has synthetic CPU measurements for shared replay snapshots, GOP boundary
caching and redundant pixel-write removal. [The review](docs/performance.md) records the results
and priorities across the workspace. Windows/AMD has generated-window GPU-surface capture and
encode evidence with zero CPU readbacks, including one-minute SDR 1080p60 measurements with fresh
and repeated images counted separately. [The report](docs/windows-performance.md) preserves the
earlier unexplained 32 fps encoder wait stall, which did not reproduce before these changes.
The two-hour generated-window 1080p30 soak passed (verified 2026-10-22); a two-hour 1080p60 soak,
real-game impact, default HDR-compatible capture and other GPUs remain open performance gates.

| ID | Status | Outcome |
| --- | --- | --- |
| M0 | 🟡 Software exit criteria met; hardware evidence open | Rust workspace, layering rules, CLI, CI gate, and the synthetic video/audio timestamp check are done; decisions are recorded in ADRs 0002–0003. Capture APIs are not yet benchmarked on reference hardware, so M0 is not marked complete |
| M1 | 🟡 Software done; Windows/macOS evidence open | Exact time primitives, audio sample math, bounded queues, a persistent rebuildable library index, MP4/MOV/MKV/WebM probing, in-app playback with sound and full controls, cached thumbnails, favorites/collections/games, and trim-and-export (no re-encode, frame-accurate start, MP4 output, progress and cancel; ADR 0010) are in place. Not done: AVI/WMV probing, playback measurements on reference hardware |
| M2 | 🟡 Linux pipeline and Windows generated-window evidence | Capture, replay, H.264 and save are tested on X11; Windows/AMD has generated-window WGC-to-MP4 runs with generated audio and one-minute native SDR 1080p60 throughput evidence. Windows hotkeys, real games, device loss and a two-hour 1080p60 soak remain unverified, so M2 is not complete |
| M3 | 🟡 In progress | Microphone and system audio as AAC tracks, mixed or separate, mute/level/push-to-talk, sync against the picture clock, device-loss recovery: done and tested with generated sources; Linux system sound (PulseAudio/PipeWire) and post-roll are done too; real-device runs and macOS system sound are open |
| M4 | 🟡 Partial | The clip-first library has search, sorting, persistence, thumbnails, favorites, collections, and game labels. Original-quality trim and local H.264 compression with estimated 10 MB, 25 MB, and custom targets are built; generated MP4/MKV tests cover decoding, audio sync, cancellation, and file preservation. Recursive scans, captured-game tagging, and advanced controls within their normal settings categories are implemented; GIF output and loudness evening are built and tested with generated clips (ADR 0018); broader export formats, WebM, and reference-machine export measurements remain open |
| M5 | 🟡 Software partial; hardware gates open | Crash/full-disk safety, resource-budget soak tools, simulated suspend/resume, keyboard focus, accessibility names, diagnostics and Linux packaging are implemented. Windows/AMD passed the two-hour generated-window 1080p30 GPU soak and has one-minute native SDR 1080p60 measurements; a two-hour 1080p60 soak, real games, device loss, HDR-compatible capture performance and broader GPU/platform support remain open. See the [release checklist](docs/release-checklist.md) |
| M6 | 🟡 Linux started | Linux: X11 capture, VA-API encoding, PulseAudio/PipeWire sound, and Wayland capture through the portal and PipeWire (ADR 0012; pieces tested, no real desktop chooser seen yet). macOS: not started |
| M7 | 🟡 Game-only arming and local game-data adapters implemented; live evidence open | Vice-style custom-first matching, 348 bundled games plus local Steam discovery, per-game exclusions, background process observation, and replay arming are implemented. Timed desktop continuation and continuous desktop/game switching are opt-in. M7.3 adds opt-in CS2/Dota 2/League match-data adapters and setup/status UI/CLI; live-game/native League TLS verification remains open. X11 has synthetic window-to-process coverage; saved clips retain the captured game name, not rich match snapshots. Real-game/Windows arming evidence and Wayland matching remain open |
| M8 | ◷ Planned | Establish tested Windows/Linux/macOS desktop support through the shared Rust platform layer; run an Android screen-capture feasibility study without promising mobile support |
| M9 | ◷ Planned | Design and implement opt-in client-side-encrypted backup to a user-selected provider, including recovery and restore; all local workflows remain offline |
| M10 | 🟡 Windows Discord artwork/IPC verified; broader live gates open | Opt-in local Discord IPC and separate game-name/match-context disclosure are built. The owned identity and CUT app icon are configured. Actual Windows CS2/Dota/League cards show Discord-hosted game icons and the small CUT badge (M10.2c), with no bundled publisher images or product HTTP client. Other-account audience, broader reconnect/macOS and live-game context checks remain open. Controller shortcuts, themes, input overlays and a permission-limited plugin/shader model remain planned |
| M11 | 🟡 Small editor implemented; broader expansion evaluation open | A bounded video/audio sequence editor adds arrangement, cuts, trim, volume/mute, undo/redo, simple fades, seven colour effects and an independent text track using FilmCraft. Preview/export share text rendering and audio fades. Sequence MP4 export offers 720p/1080p/source (up to 4K), 30/60 FPS, estimated file-size presets and GPU-encoder fallback. M11.1c adds decoded timeline strips/waveforms, prepared playback and internal local draft recovery. M11.1f bounds pending playback pictures and rejects superseded seek results; M11.1g adds cooperative playback-first decode admission; M11.1h fixes rapid percentage-seek event ordering found in native retesting. M11.1i shares bounded wgpu/CPU scaling, effects, fades and text composition between preview/export, with real software-Vulkan parity evidence; physical-GPU speed/runtime remains open. Shared decoder pooling and proxies remain open. M11.1e reduces redundant export frame work and keeps keyboard preview controls available during export; generated Linux software measurements are in the performance review. Input overlays and the MCP adapter remain planned; no full NLE, AI service or community feature is implied |

Issue-level tasks require a problem statement, explicit limitations, measurable acceptance criteria,
and test evidence. See the [`backlog`](docs/backlog.md) and
[`issue requirements`](docs/issue-requirements.md). No milestone can be marked complete from a
successful build alone; it needs end-to-end evidence for its user workflow.

Documentation ships with implementation: update `README.md` and `docs/gaps.md` in the same pull
request as every code change, then update milestone/backlog status and supporting architecture,
testing, CLI, release, or ADR guidance as needed. Pull-request CI enforces the README and gap-report
updates.

M8–M9, the remaining M10 proposals and remaining M11 expansions are not implemented capabilities. The user
authorized the portable M11.1a editor slice ahead of platform expansion; its platform claims stay
limited to the evidence in the testing guide. Each feature stays unstarted until its backlog
item's limitations, privacy model, platform matrix, and verification criteria are accepted and
evidenced. Android remains research-only, plugins require a reviewed isolation model, and backup
must encrypt before leaving the device.
