# Contributing

- **Language:** Rust only for product code, UI, engine, CLI, and required tooling. No JavaScript,
  TypeScript, Node, Electron, Tauri, web UI, non-Rust sidecar, or FFmpeg runtime.
- **Hosting:** the app is local-only. Do not add features that require a hosted backend, server
  process, account, upload, cloud sync, or share-link service.
- **Safety:** non-test code returns `Result`; no panic shortcuts. Every input-derived size and index
  is checked and bounded. Platform FFI is isolated and reviewed as described in `AGENTS.md`.
- **Commands:** implement behavior in the engine command registry; UI and CLI must dispatch the same
  command. Do not put product behavior only in a widget callback.
- **Architecture:** respect [`architecture.md`](architecture.md); `cargo xtask layers` must pass.
- **Tests:** every issue needs acceptance tests. Parsing/media/time changes cover malformed input;
  crash fixes add regression tests; capture work includes deterministic and platform tests; UI work
  includes screenshot evidence. See [`testing.md`](testing.md).
- **Visual assets:** match layout and interaction by observation, but use original or openly
  licensed art. Medal screenshots remain local references and are never product assets.
- **Issues:** follow [`issue-requirements.md`](issue-requirements.md). Each issue names limitations,
  platforms, failure behavior, acceptance criteria, and verification evidence.
- **Commits:** focused change per issue, with the stable milestone/task ID in the subject. Keep docs,
  `ROADMAP.md`, and `docs/gaps.md` consistent with implementation status.

The first Cargo workspace and automated gate are in place. Capture, playback, persistent library
metadata, editing, and export remain unimplemented; see [`gaps.md`](gaps.md).
