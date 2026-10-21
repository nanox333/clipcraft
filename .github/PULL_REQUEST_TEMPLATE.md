## User outcome

Closes #

## Scope and limitations

- Affected crates/platforms:
- Supported behavior:
- Unsupported cases and fallback:

## Required constraints

- [ ] Product and required tooling remain Rust-only; no JS/TS or non-Rust sidecar was added.
- [ ] Core workflows remain offline; any optional network use is explicitly configured, client-side encrypted where applicable, and allowed by the roadmap/ADR.
- [ ] No unsafe code outside reviewed platform FFI modules; no panic shortcuts in non-test code.
- [ ] No source clips are overwritten; memory, queues, and storage remain bounded.
- [ ] Assets are original/openly licensed and attribution is recorded.

## Verification

- [ ] Issue acceptance criteria are met with evidence.
- [ ] Relevant tests cover success, invalid input, and failure/recovery paths.
- [ ] `cargo xtask ci` passes (or explain which not-yet-implemented M0 check blocks it).
- [ ] `README.md` reflects implemented user-facing behavior and current limits.
- [ ] `docs/gaps.md` reflects remaining limitations and the evidence collected.
- [ ] The matching backlog item and `ROADMAP.md` are updated when status changed.
- [ ] Architecture, testing, CLI, release, and ADR docs are updated where their guidance changed.
- [ ] No implementation documentation is deferred to a follow-up issue.
- [ ] UI change: before/after screenshot and keyboard check attached.
- [ ] Capture/export change: deterministic test and platform/hardware evidence attached.

## Notes
