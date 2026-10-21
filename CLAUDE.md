# ClipCraft — working instructions for agents

ClipCraft is a Rust-only, offline-first desktop clip recorder and lightweight editor. Its core
workflows must not require a server process, hosted backend, account, or network connection. Optional
encrypted backup to a user-selected provider is planned separately; see `PLAN.md` and `AGENTS.md`.
Read
[`AGENTS.md`](AGENTS.md) first; those rules override this guide.

## Start each session

1. Read [`ROADMAP.md`](ROADMAP.md) for stage and current focus.
2. Read [`docs/gaps.md`](docs/gaps.md) and [`docs/backlog.md`](docs/backlog.md); choose a ranked,
   unblocked issue with clear acceptance criteria. Do not invent parity percentages or claim
   unverified work is done.
3. Read [`docs/architecture.md`](docs/architecture.md), [`docs/testing.md`](docs/testing.md), and
   the README for each crate you will change.
4. Check [`docs/issue-requirements.md`](docs/issue-requirements.md). If the issue lacks a concrete
   user outcome, limitations, acceptance criteria, or test evidence, improve the issue before
   implementation.

## Non-negotiables

- **Rust only:** no JavaScript/TypeScript, Node, Electron, Tauri, browser app, non-Rust product
  sidecar, or required non-Rust build step. Rust platform bindings are isolated in
  `clipcraft-platform`; see `AGENTS.md` for the limited `unsafe` boundary.
- **Offline first:** capture, library, editing, and export work offline. Do not add a required
  backend, account, telemetry, or share-link dependency. Optional backup is a future, opt-in,
  client-side-encrypted feature with no ClipCraft-hosted storage.
- **Never crash:** no panic shortcuts in non-test code; hostile inputs are bounded; preserve clips
  and report errors. Read `AGENTS.md` before changing unsafe or error-handling rules.
- **One command path:** add user operations to the engine command registry; UI and CLI must call the
  same implementation. There is no local control server.
- **Exact time:** use `clipcraft-time::Tick` and rational frame rates for edit and sync math.
- **Quality gate:** run `cargo xtask ci` for every implementation change. Capture/platform evidence
  still must name unsupported checks when they cannot run in the current environment.
- **Docs ship with code:** every built or changed feature updates `README.md` and
  `docs/gaps.md` in the same pull request. Update its backlog status and the roadmap when progress
  changes; update architecture, testing, CLI, release, and ADR docs wherever implementation changes
  their guidance. The PR CI checks that implementation changes include the README and gap report.
  Never leave a finished feature documented as planned or incomplete.
- **Visual work:** run the desktop app, inspect the actual result, and attach before/after screenshots
  to the change. Match the Medal layout and visual hierarchy with original assets only.

## Issue and commit workflow

Take the highest-ranked issue that has no active owner and no unresolved dependency. Keep each
change focused on one issue; include the stable issue ID in the branch or commit subject. Update the
gap entry, roadmap status, README feature list, backlog status, tests, and relevant docs in the same
change as implementation. Do not ask the user to make routine implementation choices; record a
decision in an ADR when it changes a product constraint or public format.
