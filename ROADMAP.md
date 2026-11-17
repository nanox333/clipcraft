# ClipCraft roadmap

**Stage:** early release (0.4.0), working toward the first stable release · **Last reviewed:** 2026-11-17 · **Target:** offline-first, game-aware capture and local editing

ClipCraft is a pure-Rust, offline-first native desktop product. Capture, library, editing, and export
must work offline. Optional user-configured client-side-encrypted backup is planned, with no
ClipCraft-hosted storage or account. The complete direction and constraints are in
[`PLAN.md`](PLAN.md).

## Goal: the first stable release (1.0)

Everything on this page serves one goal: a stable **1.0 release for Windows** that people can
install, trust with their clips, and update without worry. Linux follows if its gates pass;
macOS is not needed for 1.0.

1.0 is ready when the local alpha and beta gates below pass **and**:

- a signed installer installs, upgrades from the previous release and uninstalls cleanly on a
  machine that never had ClipCraft;
- there is no known bug that can lose a clip, and the known-limits list in the
  [gap report](docs/gaps.md) matches what the program really does;
- the release workflow builds the packages from the tag, and the downloaded packages themselves
  have been run.

The path, in the order I suggest working on it. Each step names the gate it closes.

| # | Step | Closes | Status |
| --- | --- | --- | --- |
| 1 | One written Windows pass: record a real game, press the hotkey, trim, export, with the network off | Local alpha | ⬜ Not done |
| 2 | Windows finds the game window reliably (fullscreen, borderless, launcher games), checked on real games | Local alpha | 🟡 Fixes in 0.4.0, not yet verified on real games |
| 3 | Real-game performance and a two-hour 1080p60 soak, on at least two graphics-card makers (proposed; so far one AMD card and generated footage) | Beta | ⬜ Not done |
| 4 | Recovery when a device disappears (microphone, monitor) and when the disk fills | Beta | 🟡 Low-disk warning and backups built; device loss open |
| 5 | Accessibility review with a real screen reader | Beta | 🟡 Names and keyboard focus tested; no review |
| 6 | Signing key set, signed installer and signed update check | Beta / 1.0 | ⬜ Not done (downloads are unsigned) |
| 7 | Clean-machine install, upgrade from the previous release and uninstall | 1.0 | ⬜ Not done for 0.4.0 |
| 8 | Run the published packages and the command line release; fix what breaks | 1.0 | ⬜ Not done |
| 9 | Decide whether Linux (X11 and Wayland) ships as supported or as experimental in 1.0 | 1.0 | ◷ Open decision |

New features wait unless they serve this list. After 1.0: a macOS capture provider (M6),
optional client-side encrypted backup (M9; the rolling backups in 0.4.0 are local copies of the
settings and library index, not this), controller shortcuts and overlays, and more editor polish.

## Current focus (2026-11-17)

Steps 1 to 3 above. The first two need real Windows machines and real games; the
[gap report](docs/gaps.md) lists every unverified item. Anyone with a Windows PC can help by
testing and sending the bug report from Settings → About.

## What is built (0.4.0)

The important features that exist today. "Evidence" says how far each was checked; the
[gap report](docs/gaps.md) has the detail and the limits. Real games are not yet covered anywhere.

| Area | Built | Evidence |
| --- | --- | --- |
| Capture and replay | Screen or window capture into a bounded replay buffer; save the last moments with a global hotkey (up to five shortcuts per action, shortcuts for different clip lengths); pause and resume; post-roll; automatic quality and a safety watchdog that protect your computer; hardware H.264/H.265 on supported Windows GPUs and a built-in software H.264 encoder | Windows/AMD and Linux/X11 with generated captures; two-hour generated soak at 1080p30 |
| Game awareness | Game-only mode that follows a recognized game (348 bundled games plus local Steam discovery), desktop fallback when no game window is found, per-game exclusions | Logic and unit tests; no real-game evidence yet |
| Audio | System sound and microphone on mixed or separate tracks, sound of one program on its own track, optional offline noise suppression, push-to-talk | Generated audio on Windows and Linux; real games open |
| Library | Search, sort, favorites, collections, game labels and colours, a heading for each day, cards or a list, your own order by dragging, rename, thumbnails, choose many clips at once, Refresh that checks every clip | Linux with generated clips; Windows Recycle Bin and mouse selection not yet run |
| Editor and export | Trim, split, arrange, fades, colour effects, styled text, frame stepping, timeline zoom, undo/redo, draft recovery, preview and export in any picture shape (fit, blur, fill, stretch), 720p/1080p/source at 30 or 60 FPS, size targets, GIF, loudness evening | Real software exports on Linux; physical GPU not checked |
| Safety | Recycle Bin with an Undo message instead of deleting, storage limit that spares favorites, every saved clip checked, low-disk warning, rolling backups of settings and library, safe start after failed starts, question before quitting with footage, crash-safe saves and recovery | Unit tests and Linux; Windows behaviour not yet verified |
| Sharing | Drag a clip into Discord or any program, copy file, export sized for Discord, optional Discord Rich Presence | Linux and tests; Windows drag-and-drop not yet run |
| Privacy | Fully offline, no account or telemetry, optional update check you can turn off; experimental pause for password managers, programs you name and the lock screen | Linux X11 tested with a real window |
| App and release | Windows installer and portable zip, Linux tarball, system tray, one copy at a time, bug-report helper, one message design, the command line tool with the same commands | 0.4.0 published; packages not yet run here, unsigned |

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
- **Release (1.0):** the local alpha and beta gates pass, the installer is signed and installs,
  upgrades and uninstalls cleanly on a clean machine, the published packages have been run, and
  no known bug can lose a clip.
- **Additional platforms:** macOS and Linux providers pass the same local capture, audio, recovery,
  and export contract.

## Milestones

M5.B1 adds reviewable local bug-report preparation and a separately opened GitHub form. The
report is never submitted automatically; physical Windows/macOS UI evidence remains open.

M2.R1 adds capture-bar replay availability and notices when source/stream changes clear buffered
video. Simulated reset checks and an X11 virtual-desktop visual/keyboard check cover the first
slice; real GPU, Windows, Wayland and game-transition evidence remain open.

M6.S2 tries the Wayland GlobalShortcuts portal first, shows actual assignments and offers manual
retry; direct keyboard-device fallback is explicitly opt-in. Fake-bus integration passes; real
Wayland desktop and physical-device checks remain open.

M5.1a adds up to five bindings per shortcut action, first-run setup and an offline per-user Rust
Windows installer alongside the portable zip. Signing and clean-machine release checks remain open.

M10.2b establishes the maintainer-selected A / CUT identity, with shared native/Discord product
artwork and a reproducible brand kit. M10.2c replaces three typographic game placeholders with
Discord-hosted icon references; no publisher image files or runtime lookup service are bundled.
Discord upload and profile rendering are separate verification steps.

M2.P2 fixes idle-focus redraws, caches library views and virtualizes cards, and serializes index
writes off the UI with explicit retry/shutdown behavior. Generated/cloud measurements and limits
are in [the performance review](docs/performance.md); broader audit proposals remain planned.

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
| M3 | 🟡 In progress | Microphone and system AAC, mixed/separate tracks, mute/level/push-to-talk, picture-clock sync and generated recovery are built. M3.1a adds default-off offline RNNoise suppression; M3.1b preserves stereo with optional mono and adds processed microphone listening/metering with live settings. Generated MP4/MKV and a short Windows input-adapter check pass. Linux system sound and post-roll are done; real speech quality, physical-device recovery and macOS system sound remain open |
| M4 | 🟡 Partial | The clip-first library has search, sorting, persistence, thumbnails, favorites, collections, and game labels. Original-quality trim and local H.264 compression with estimated 10 MB, 25 MB, and custom targets are built; generated MP4/MKV tests cover decoding, audio sync, cancellation, and file preservation. Recursive scans, captured-game tagging, and advanced controls within their normal settings categories are implemented; GIF output and loudness evening are built and tested with generated clips (ADR 0018); broader export formats, WebM, and reference-machine export measurements remain open. The clips page was redone in 0.4.0: a heading for each day, picture cards in three sizes or a list, your own order by dragging, a colour and mark per game, and a **Refresh** that rescans the folder and checks every clip. **Select** chooses many clips (click, Shift-click, a drawn box, Ctrl and A) to favorite, collect or delete together; every delete, of one clip or many, goes to the Recycle Bin behind an Undo message that stays until closed. Checked on Linux with generated clips; the Recycle Bin and mouse selection on Windows were only type-checked and linted, see the [gap report](docs/gaps.md) |
| M5 | 🟡 Software partial; hardware gates open | Crash/full-disk safety, resource-budget soak tools, simulated suspend/resume, keyboard focus, accessibility names, diagnostics and Linux packaging are implemented. Windows/AMD passed the two-hour generated-window 1080p30 GPU soak and has one-minute native SDR 1080p60 measurements; a two-hour 1080p60 soak, real games, device loss, HDR-compatible capture performance and broader GPU/platform support remain open. See the [release checklist](docs/release-checklist.md). 0.4.0 adds rolling backups of the settings and library, a start with default settings after repeated failed starts, a check of every saved clip, a low-disk warning, a storage limit that leaves favorites, collections and clips in an open edit alone, and a question before quitting with footage in the replay. Experimental and off by default: pausing while a password manager, a program you name or the lock screen is up. These were tested on Linux and by unit tests; Windows behaviour (Recycle Bin, process and lock-screen detection) is not yet verified |
| M6 | 🟡 Linux started | Linux: X11 capture, VA-API encoding, PulseAudio/PipeWire sound, and Wayland capture through the portal and PipeWire (ADR 0012; pieces tested, no real desktop chooser seen yet). macOS: not started |
| M7 | 🟡 Game-only arming and local game-data adapters implemented; live evidence open | Vice-style custom-first matching, 348 bundled games plus local Steam discovery, per-game exclusions, background process observation, and replay arming are implemented. Timed desktop continuation and continuous desktop/game switching are opt-in. M7.3 adds opt-in CS2/Dota 2/League match-data adapters and setup/status UI/CLI; live-game/native League TLS verification remains open. X11 has synthetic window-to-process coverage; saved clips retain the captured game name, not rich match snapshots. Real-game/Windows arming evidence and Wayland matching remain open |
| M8 | ◷ Planned | Establish tested Windows/Linux/macOS desktop support through the shared Rust platform layer; run an Android screen-capture feasibility study without promising mobile support |
| M9 | ◷ Planned | Design and implement opt-in client-side-encrypted backup to a user-selected provider, including recovery and restore; all local workflows remain offline |
| M10 | 🟡 Windows Discord artwork/IPC verified; broader live gates open | Opt-in local Discord IPC and separate game-name/match-context disclosure are built. The owned identity and CUT app icon are configured. Actual Windows CS2/Dota/League cards show Discord-hosted game icons and the small CUT badge (M10.2c), with no bundled publisher images or product HTTP client. Other-account audience, broader reconnect/macOS and live-game context checks remain open. Controller shortcuts, themes, input overlays and a permission-limited plugin/shader model remain planned |
| M11 | 🟡 Small editor implemented; broader expansion evaluation open | A bounded video/audio sequence editor adds arrangement, cuts, trim, volume/mute, undo/redo, simple fades, seven colour effects and an independent text track using FilmCraft. Preview/export share text rendering and audio fades. Sequence MP4 export offers 720p/1080p/source (up to 4K), 30/60 FPS, estimated file-size presets and GPU-encoder fallback. M11.1c adds decoded timeline strips/waveforms, prepared playback and internal local draft recovery. M11.1f bounds pending playback pictures and rejects superseded seek results; M11.1g adds cooperative playback-first decode admission; M11.1h fixes rapid percentage-seek event ordering found in native retesting. M11.1i shares bounded wgpu/CPU scaling, effects, fades and text composition between preview/export, with real software-Vulkan parity evidence; physical-GPU speed/runtime remains open. M11.1j removes duplicate UI text textures/rendering and dormant utilities while retaining shared composition and drag geometry. Shared decoder pooling and proxies remain open. M11.1e reduces redundant export frame work and keeps keyboard preview controls available during export; generated Linux software measurements are in the performance review. Input overlays and the MCP adapter remain planned; no full NLE, AI service or community feature is implied. 0.4.0 adds a transport row with frame stepping by each clip's own frame rate, timeline zoom and a segment panel; preview and export in twelve picture shapes or any 1:4 to 4:1 with fit, blur, fill or stretch; and styled text (three font families, four weights, italic, underline, capitals, spacing, shadow, outline, box, opacity, six styles). Verified with real software exports on Linux; a physical GPU, styled text in an exported MP4 and a vertical export played elsewhere are not yet checked |

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
