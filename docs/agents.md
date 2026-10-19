# Driving ClipCraft from agents

ClipCraft is designed so the UI and CLI use the same engine commands. A feature that works only by
clicking a widget is incomplete. Stable command IDs, typed parameters, validation, and error results
make behavior testable and scriptable without a network service.

The interfaces are:

| Interface | Use | Requires a server? |
| --- | --- | --- |
| Desktop UI | Normal capture, library, edit, settings, and export workflow | No |
| `clipcraft-cli` | Current: list commands, scan a local folder, and print a local file path; planned: inspect, edit, and export | No |
| Engine tests | Deterministic command-level and workflow verification | No |

Do not add an HTTP/WebSocket control endpoint, MCP server, account, or remote automation service.
Commands that affect files must validate paths, avoid overwriting inputs, and report partial or
unknown outcomes without silently retrying destructive operations.

Before work, read [`CLAUDE.md`](../CLAUDE.md), [`AGENTS.md`](../AGENTS.md), the ranked
[`gaps.md`](gaps.md), and [`issue-requirements.md`](issue-requirements.md). The CLI currently exposes
`commands`, the local `library scan` operation, the local `library path` operation, and `library reveal` (show a file in the system file manager). It has no
capture, edit, playback, or export commands yet.
