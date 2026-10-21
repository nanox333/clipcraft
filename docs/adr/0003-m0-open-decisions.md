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
| Storage quota defaults | The shipped default has no saved-clip storage limit. The user may set a limit and choose to delete only ClipCraft-owned clips or stop saving; a save also checks available space, and mid-save full disk fails without removing existing clips. A separate minimum-free-space pause threshold from the original proposal was not implemented. | Revisit if storage policy changes |
| First hardware encoder matrix | H.264 is the only recording codec. Software H.264 and hardware adapter paths for NVENC, VideoToolbox, Media Foundation, and VA-API are wired; hardware has only been tested against stand-ins. | M2/M5 real-device evidence |
| Source selection | First run preselects the primary monitor; window capture is chosen from a picker. Game-specific hooks and injection are out of scope. | M2 |
| Offline-first constraint | Core capture/library/edit/export work offline with no hosted dependency. Optional user-configured, client-side-encrypted backup is permitted by the ADR 0001 amendment; details wait for M9 | M9 design ADR |

## Consequences

Deferrals are deliberate: an unmeasured number is recorded as provisional rather than presented as
a fact. Each deferral blocks the milestone named in the last column, not M0.
