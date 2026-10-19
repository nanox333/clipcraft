## User outcome

Closes #

## Scope and limitations

- Affected crates/platforms:
- Supported behavior:
- Unsupported cases and fallback:

## Required constraints

- [ ] Product and required tooling remain Rust-only; no JS/TS or non-Rust sidecar was added.
- [ ] No hosted service, server process, network dependency, account, upload, or sync was added.
- [ ] No unsafe code outside reviewed platform FFI modules; no panic shortcuts in non-test code.
- [ ] No source clips are overwritten; memory, queues, and storage remain bounded.
- [ ] Assets are original/openly licensed and attribution is recorded.

## Verification

- [ ] Issue acceptance criteria are met with evidence.
- [ ] Relevant tests cover success, invalid input, and failure/recovery paths.
- [ ] `cargo xtask ci` passes (or explain which not-yet-implemented M0 check blocks it).
- [ ] `docs/gaps.md`, roadmap, architecture, and crate docs are updated as needed.
- [ ] UI change: before/after screenshot and keyboard check attached.
- [ ] Capture/export change: deterministic test and platform/hardware evidence attached.

## Notes
