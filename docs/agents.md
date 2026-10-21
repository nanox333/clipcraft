# Driving ClipCraft from agents

ClipCraft's desktop UI and CLI use the same engine commands so behavior can be tested and scripted
without a server. Keep command IDs stable, validate typed parameters in the engine, and report
actionable errors.

| Interface | Current use | Requires a server? |
| --- | --- | --- |
| Desktop UI | Capture status and settings, local recording, library organization, playback, trim/export, diagnostics | No |
| `clipcraft-cli` | List commands; inspect, scan, reveal, and thumbnail local clips; view recording settings; list capture targets; record; trim; run diagnostics | No |
| Engine/platform tests | Deterministic command, media, capture, audio, and failure/recovery verification | No |

The CLI's main command groups are `library`, `recording`, `capture`, `clip`, and `diagnose`. Use
`clipcraft-cli help` for exact arguments. Commands read and write local files only; they do not
start an HTTP/WebSocket control endpoint, MCP server, or remote automation service.

## Documentation is part of the change

Before implementation, read [`CLAUDE.md`](../CLAUDE.md), [`AGENTS.md`](../AGENTS.md), the current
[`ROADMAP.md`](../ROADMAP.md), [`gaps.md`](gaps.md), [`backlog.md`](backlog.md), and
[`issue-requirements.md`](issue-requirements.md).

Whenever behavior is built or changed, update `README.md` and `docs/gaps.md` in the same pull
request. Update the matching backlog item and roadmap status when progress changes, plus relevant
architecture, testing, CLI, release, and ADR documents. The PR CI checks the required README and gap
report changes for code modifications under `apps/`, `crates/`, or `tools/`.
