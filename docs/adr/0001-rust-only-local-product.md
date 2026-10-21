# ADR 0001: Rust-only, local desktop product

- **Status:** Accepted; offline-core constraint amended 2026-10-21
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
platform crate. Do not require a hosted backend, user account, network connection, or server process
to install, launch, capture, manage clips, edit, or export.

Use ordinary video files as the primary user artifacts. Keep only rebuildable local library metadata
and caches. The MVP has no custom `.ccproj` file or save-project workflow. Its editor is limited to
preview, cut/split, trim, undo for the active edit, and export to a new local video file.

Medal's public pages and user-supplied screenshots are visual and workflow references. ClipCraft
will use original code, labels, icons, and assets; it will not copy Medal's implementation or
proprietary media.

### Amendment: optional encrypted backups

The user has requested encrypted cloud backups. This amends the earlier absolute ban on cloud
features: a future, explicitly configured backup may send client-side-encrypted clip data to a
provider selected by the user. The core workflows above remain available offline; ClipCraft does not
operate a hosted service, require an account with ClipCraft, or store encryption keys. The storage
provider, cryptographic format, key recovery, metadata exposure, and restore/deletion semantics are
not yet decided and must be specified in the corresponding backlog issue before implementation.

## Consequences

- Local capture, browsing, storage controls, details, basic edits, and export are the acceptance
  center; sharing is done by the user with exported files outside ClipCraft.
- No backup feature is implemented yet. Index loss must remain recoverable by rescanning local
  media folders. A future backup is opt-in and is not equivalent to library sharing or automatic
  cross-device sync.
- Native capture and encoder APIs may need Rust bindings and isolated platform-specific `unsafe`.
- Codec, licensing, installation, crash recovery, resource limits, and platform support need explicit
  verification before a release claim.
- The pure-Rust requirement, no-required-hosted-service rule, ordinary-video primary artifacts, and
  quick-cut MVP editor remain constraints. Further changes require an ADR and updates to
  `AGENTS.md`, `PLAN.md`, and the roadmap.
