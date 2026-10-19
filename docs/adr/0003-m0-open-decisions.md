# ADR 0003: M0 open decisions

- **Status:** Accepted for M0; each item names the milestone that may supersede it.
- **Date:** 2026-10-19
- **Decision owners:** ClipCraft maintainers

`PLAN.md` lists open decisions to resolve at M0. This ADR records the answer or the explicit
deferral for each so later work has one place to check.

| Decision | Outcome | Revisit |
| --- | --- | --- |
| Minimum Windows version | Windows 10 2004 (build 19041) and Windows 11; see ADR 0002 | M3 audio spike |
| FilmCraft reuse | **Superseded by [ADR 0004](0004-filmcraft-crates.md):** consume FilmCraft crates as pinned git dependencies and adopt its tick clock. Never fork a large editor wholesale. | See ADR 0004 |
| Library metadata / index format | A versioned JSON file in the per-user app data directory, written atomically (temp file, flush, rename), always rebuildable by rescanning the configured folders. Missing files show as offline entries. SQLite is rejected for now because it adds a C dependency to a Rust-only product. | M4.1, if scans exceed `MAX_LIBRARY_RESULTS` (5 000) |
| Storage quota defaults | Provisional: replay buffer capped at 2 GiB of disk, no cap on saved clips, recording pauses with a visible error below 5 GiB free disk. Users can change all three. | M5, after full-disk tests |
| First hardware encoder matrix | H.264 through Windows Media Foundation hardware MFTs; software fallback chosen in the M2 spike (ADR 0002) | M2 |
| Source selection | First run preselects the primary monitor; window capture is chosen from a picker. Game-specific hooks and injection are out of scope. | M2 |
| Local-only constraint | No hosted component of any kind; see ADR 0001 | Superseding ADR only |

## Consequences

Deferrals are deliberate: an unmeasured number is recorded as provisional rather than presented as
a fact. Each deferral blocks the milestone named in the last column, not M0.
