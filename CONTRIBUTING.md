# Contributing to ClipCraft

ClipCraft is a local-first Rust desktop clip recorder and lightweight editor. Read
[`AGENTS.md`](AGENTS.md) for binding project rules, then [`CLAUDE.md`](CLAUDE.md) for the working
process. The current capabilities and platform evidence are listed in [`README.md`](README.md),
[`ROADMAP.md`](ROADMAP.md), and [`docs/gaps.md`](docs/gaps.md).

## Product and implementation limits

- Keep first-party application code and required tools in the Rust workspace. Native OS APIs are
  accessed through Rust platform adapters.
- The core flow must work offline without an account, server, backup provider, telemetry, or
  share-link service. Optional encrypted backup is planned as an explicitly configured feature; see
  the requirements in `docs/issue-requirements.md`.
- Keep capture and library as the main product. The MVP editor is playback, trim, and cut; a focused
  one-track expansion is a later proposal, not a multitrack-editor commitment.
- ClipCraft reuses pinned open-source FilmCraft crates. Review license, API, and safety implications
  for dependencies and record material decisions in an ADR. Never copy proprietary source or assets.
- Preserve original media, bound resource use, treat input files as hostile, and return actionable
  errors. Keep `unsafe` in the reviewed platform boundary.

## Issues, implementation, and documentation

Every implementation starts from a stable backlog item and GitHub issue. Specify the user outcome,
evidence, supported platforms, requirements, limitations/fallbacks, acceptance criteria, verification,
and documentation that must change.

**Documentation is part of the implementation, in the same pull request.** Every code change under
`apps/`, `crates/`, or `tools/` must update `README.md` and `docs/gaps.md`. Also update the matching
backlog item and `ROADMAP.md` when status changes; update architecture, testing, CLI, release, and
ADR documentation wherever implementation changes their guidance. Do not leave shipped behavior
described as planned or unimplemented. The CI documentation check enforces the README and gap-report
updates for implementation changes.

Run `cargo xtask ci` and include its result in the pull request. Capture and platform work must name
the evidence that could not be collected in the current environment. See
[`docs/testing.md`](docs/testing.md) and [`docs/issue-requirements.md`](docs/issue-requirements.md).

## Pull requests

Describe user-visible behavior, limits, and test evidence. Include screenshots for UI changes using
generated or licensed sample media. Never commit Medal reference screenshots or private gameplay.
Update `ATTRIBUTION.md` for every shipped non-code asset. Do not mark a feature complete until its
documentation and acceptance evidence are present.
