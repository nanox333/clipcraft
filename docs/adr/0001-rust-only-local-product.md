# ADR 0001: Rust-only, local desktop product

- **Status:** Accepted as the product constraint
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

## Context

ClipCraft takes its product direction from Medal's desktop capture and library workflow. The target
is an app that runs on the player's own computer, saves ordinary local video files, and remains
useful without a hosted account or service. ArtCraft engineering practices call for explicit
architecture decisions, testable constraints, and local automation.

## Decision

Implement first-party application code, UI, CLI, and required build/test tooling in Rust in a Cargo
workspace. Use a native Rust desktop UI and isolate any necessary OS API bindings behind the Rust
platform crate. Do not require a hosted backend, user account, upload, sync, telemetry endpoint,
network connection, or server process to install, launch, capture, manage clips, or export.

Use ordinary video files as the primary user artifacts. Keep only rebuildable local library metadata
and caches. The MVP has no custom `.ccproj` file or save-project workflow. Its editor is limited to
preview, cut/split, trim, undo for the active edit, and export to a new local video file.

Medal's public pages and user-supplied screenshots are visual and workflow references. ClipCraft
will use original code, labels, icons, and assets; it will not copy Medal's implementation or
proprietary media.

## Consequences

- Local capture, browsing, storage controls, details, basic edits, and export are the acceptance
  center; sharing is done by the user with exported files outside ClipCraft.
- There is no cloud backup or cross-device sync. Index loss must be recoverable by rescanning local
  media folders.
- Native capture and encoder APIs may need Rust bindings and isolated platform-specific `unsafe`.
- Codec, licensing, installation, crash recovery, resource limits, and platform support need explicit
  verification before a release claim.
- Any proposed change to the language, hosted-service, artifact, or editor constraints requires a
  superseding ADR and updates to `AGENTS.md`, `PLAN.md`, and the roadmap.
