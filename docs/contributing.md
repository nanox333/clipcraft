# Contributing

- **Language:** Rust only for product code, UI, engine, CLI, and required tooling. No JavaScript,
  TypeScript, Node, Electron, Tauri, web UI, non-Rust sidecar, or FFmpeg runtime.
- **Offline first:** core workflows work without a network, account, or server process. Optional
  encrypted backup to a user-selected provider is planned; it must encrypt on-device, be explicitly
  configured, and never depend on ClipCraft-hosted storage. See [`issue-requirements.md`](issue-requirements.md).
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
- **Open-source reuse:** ClipCraft uses pinned FilmCraft crates. Review source, license, API, and
  safety implications before changing dependencies; record material decisions in an ADR. This is
  independent product code with reviewed open-source reuse, not a formal clean-room implementation.
- **Issues:** follow [`issue-requirements.md`](issue-requirements.md). Each issue names limitations,
  platforms, failure behavior, acceptance criteria, and verification evidence.
- **Documentation:** update `README.md` and `docs/gaps.md` in the same pull request as every code
  change under `apps/`, `crates/`, or `tools/`. Update the matching backlog item and `ROADMAP.md`
  when status changes, and update architecture/testing/CLI/release/ADR docs as their guidance
  changes. The PR CI enforces README and gap-report changes; documentation is not deferred follow-up
  work.
- **Commits:** focused change per issue, with the stable milestone/task ID in the subject. Keep
  documentation, `ROADMAP.md`, and `docs/gaps.md` consistent with implementation status.

The current implementation and its evidence are summarized in [`README.md`](../README.md) and
[`ROADMAP.md`](../ROADMAP.md); remaining limitations are tracked in [`gaps.md`](gaps.md).
