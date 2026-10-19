# Known gaps

**Status:** active build baseline · **Last reviewed:** 2026-10-19 · **Target:** local Windows capture-to-export alpha

This is the ranked work list, not a parity claim. The workspace, UI shell, time primitives, scanner,
and first command tests exist; capture-to-export is still unimplemented. Estimates and performance
claims remain TBD until the platform spikes provide evidence. Rank by user impact, frequency, and
evidence. Update this file whenever a gap's status, workaround, or estimate changes.

| Rank | Gap | Evidence / current limitation | Impact | Work |
| ---: | --- | --- | --- | --- |
| G0 | M0 hardware evidence and Windows CI remain open | Workspace, shell, CLI, time crate (including audio sample math and a one-hour synthetic A/V alignment test), scanner, and layer/asset/license checks exist; the gate passes locally on Linux. The capability matrix ([ADR 0002](adr/0002-capture-platform-matrix.md)) is documentation only, and the Windows CI job has not run | Limits platform and release claims | M0 close-out |
| G1 | Windows capture API and hardware matrix are undecided | Must measure monitor/window capture, audio APIs, permissions, and encoder availability | Core app cannot record reliably | M0 spike, then M2/M3 |
| G2 | Replay buffer and A/V synchronization are unimplemented | Exact rational time primitives exist; no segment ring, bounded queues, or A/V synchronization tests | Instant clipping is the core workflow | M1/M2/M3 |
| G3 | Persistent library and storage controls are incomplete | The desktop can browse one local folder, scan it, search titles, sort clips, copy paths, and reveal files; a versioned local index restores the last folder; MP4/MOV duration and stream facts are probed; clips play with sound; thumbnails are cached. Missing: WebM/MKV probing, a storage quota, full-disk behavior, and any UI for the stored favorites and tags | Users still have no durable library organization or storage protection | M1.2/M4.1/M5 |
| G4 | Editor and export quality are unverified | No cut/trim model, MP4 exporter, or pinned media corpus | Cannot deliver dependable clips | M1/M4 |
| G5 | Mac and Linux capture are not implemented | No ScreenCaptureKit/CoreAudio or PipeWire/portal provider | Initial release is Windows-only | M6 |

G0 tracks remaining workspace and gate work. The capture capability spike in G1 is the next blocker
for recording features. Do not mark a gap done because code compiles; attach evidence for its user
workflow and required failure cases. Do not publish percentages until a repeatable scorecard exists.

## Recording output options

- **Done:** recording preferences (container MP4/MKV, game or desktop source, 16:9 / 4:3 / source shape with stretch, black bars, or crop, resolution, frame rate, replay length) are modelled, validated, saved locally (`recording.load_settings` / `recording.save_settings`, `clipcraft-cli recording settings`), and shown in Settings. The reframe geometry is unit-tested.
- **WebM unavailable:** FilmCraft has VP9 and Opus decoders only. Pure-Rust encoders exist on crates.io (`rusty_vp9`, `opus-pure`, `rav1e`) but are young, carry many `unsafe` blocks, and have not been vetted. WebM stays greyed out until one passes a spike (decode check with ffprobe/ffmpeg) and a licence and `unsafe` review.
- **Not done:** capture itself (full-screen game and desktop sources), and exporting a clip with the chosen format and framing. Neither can be verified in a headless container.
