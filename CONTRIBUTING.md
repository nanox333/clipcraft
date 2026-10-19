# Contributing to ClipCraft

ClipCraft is a capture-first, local desktop app. Read [`AGENTS.md`](AGENTS.md) for the binding
requirements before proposing code, then read [`CLAUDE.md`](CLAUDE.md) for the issue workflow.
The repository is in the early build stage. Capture, playback, persistent library metadata, editing,
and export are not implemented yet.

## Product and implementation limits

- Write all first-party app code and required build/test tooling in Rust. The product is a native
  desktop app and must not require a hosted service, account, network connection, or server process.
- Keep the user flow close to Medal's desktop capture and clip-library experience: capture status,
  dense clip grid, clip details, storage/audio settings, and local file actions. Use original artwork
  and generated or openly licensed media.
- Capture and library are the main product. The editor supports quick preview, trim, cut/split, and
  export; do not expand it into a multitrack editor.
- Preserve original media. Treat inputs as hostile, bound memory and disk work, return actionable
  errors, and keep `unsafe` inside the reviewed platform boundary.

## Issues and changes

Every implementation starts from a stable backlog item and GitHub issue. State the user outcome,
evidence, supported platforms, requirements, limitations/fallbacks, acceptance criteria, and the
verification for each criterion. Keep changes reviewable and focused; update the roadmap and ranked
gap record when evidence or scope changes.

The single quality gate is `cargo xtask ci`. Run it and include its result in the pull request. Be
explicit when requested platform checks are unavailable in the current environment. See
[`docs/testing.md`](docs/testing.md) for required behavior and failure tests.

## Pull requests

Describe the user-visible change, constraints, and test evidence. Include screenshots for UI changes
using generated or licensed sample media; never commit Medal reference screenshots or private
gameplay. List remaining limitations and update `ATTRIBUTION.md` for every shipped non-code asset.
