# Issue requirements

Issues are the project work contract. Keep them small enough to review and test, and specific
enough that a contributor can finish without guessing product behavior. The GitHub issue forms in
`.github/ISSUE_TEMPLATE/` collect these fields.

## Non-negotiable limitations for every issue

Every issue must preserve these constraints or explicitly propose a product change for user review:

1. **Rust-only product:** first-party app, UI, engine, CLI, local library metadata, and required tooling are
   Rust. No JavaScript/TypeScript, Node, Electron, Tauri, web frontend, non-Rust product sidecar, or
   required non-Rust build step. OS APIs may be called through Rust bindings in `clipcraft-platform`.
2. **Offline-first:** core features work offline with no hosted backend, account, or server process.
   Optional encrypted backup may use a user-configured provider, but must not gate capture,
   library, editing, or export. No social feed, share links, or telemetry.
3. **Local media:** never transmit capture media by default. The only planned exception is an
   explicitly configured backup that encrypts media on-device before transfer. Do not overwrite
   source clips. Bound storage, memory, queues, and processing time; use atomic output and actionable
   full-disk errors.
4. **Safe Rust boundary:** no panic shortcuts in non-test code. Keep `unsafe` out of all crates except
   the explicitly reviewed platform FFI modules; every such exception needs an ADR and safety proof.
5. **Code and asset provenance:** public behavior/specifications are references. Do not copy
   proprietary code, artwork, game footage, logos, screenshots, or private APIs. A source port from
   an open-source project requires a compatible license or explicit permission for the intended
   adaptation and distribution; record the exact revision, source files, scope, and attribution.
   Shipped non-code assets need a compatible license and an entry in `ATTRIBUTION.md`.
6. **Layering and time:** lower crates cannot depend on UI/platform layers; edit math uses integer
   ticks and rational frame rates.

An issue that proposes an exception must identify it as a product-direction change and link the
relevant ADR/backlog item. Do not quietly add network use, native plugin loading, background game
capture, or a new platform promise inside an unrelated task.

An issue that conflicts with a constraint is not ready for implementation. Do not quietly weaken a
rule inside an implementation issue.

## Required issue fields

Public bug reports may initially contain only the user's description and whatever details they
know. Triage fills in missing engineering fields before the issue is treated as ready for work;
reporters are not expected to know internal milestone IDs or regression-test plans.

Every issue, including chores and documentation tasks, must contain:

- **Stable ID and milestone:** for example `M2.3` or `G1`; link the corresponding ranked gap or
  roadmap item. GitHub's issue number is also recorded when one exists.
- **Problem and impact:** who is affected and what they cannot do. For bugs, include observed
  behavior and reproducible steps; for features, state the user outcome.
- **Evidence:** logs with private values removed, a minimal synthetic case, screenshot, measurement,
  or public specification. Mark assumptions when evidence is unavailable.
- **Scope:** crates/platforms involved, desired behavior, explicit non-goals, and unsupported cases.
- **Limitations:** what the change intentionally will not support, which platform/API constraints
  remain, and what error or fallback the user sees.
- **Acceptance criteria:** checkable statements that describe observable behavior, including at
  least one success path and relevant failure/recovery paths.
- **Verification:** tests, platform hardware, visual checks, and commands required to prove each
  criterion. Do not write “test it” without saying how.
- **Documentation impact:** identify the status and user guidance that will change. For every
  implementation under `apps/`, `crates/`, or `tools/`, update `README.md` and `docs/gaps.md` in the
  same pull request. Name any matching backlog, roadmap, architecture, testing, CLI, release, or ADR
  pages that also need updates.
- **Dependencies and estimate:** blocking work, risk, and a rough effort estimate or `TBD at M0`.

If a field does not apply, write why. Do not leave it blank.

## Type-specific requirements

### Bug

- Include expected and actual behavior, exact reproduction steps, app commit/version, OS, capture
  source type, and relevant GPU/audio hardware. Never attach private clips without redaction/consent.
- State whether the bug loses data, blocks capture, affects export correctness, or has a workaround.
- Add a regression test that fails before the fix. If not automatable, provide a repeatable manual
  verification procedure and explain the blocker.

### Feature

- Describe the complete user flow, defaults, keyboard access, empty/loading/error states, and
  explicit non-goals. Name the engine command(s) and any impact on library metadata or source media.
- Add unit/property/integration coverage appropriate to the layer. UI work includes before/after
  screenshots; capture work includes deterministic fake-provider tests and target-platform evidence.

### Platform work

- Name OS version, API and permission prompts, unsupported windows/sources, hardware encoder/audio
  requirements, fallback behavior, and real-device test evidence.
- Keep platform-specific code behind the safe platform trait. Never require a remote service to
  work around missing OS support.
- For Android, name the MediaProjection consent flow, foreground-service lifetime, background and
  game-detection limits, and battery/thermal test plan. A desktop compile does not establish Android
  support.

### Encrypted backup work

- Specify the supported storage provider(s), authentication and local secret storage, key/passphrase
  creation and recovery, ciphertext format/versioning, and exactly which metadata remains visible to
  the provider.
- Encrypt locally with a reviewed Rust cryptography library before any upload; never invent a cipher
  or send clear media to the provider. Backups are disabled until the user chooses a destination and
  confirms setup. No ClipCraft-hosted account, bucket, server, or key escrow is allowed.
- Define offline behavior, interruption/retry, integrity verification, restore to a fresh install,
  deletion semantics, credential revocation, and what happens when a key is lost. Never log tokens,
  keys, clip paths, or decrypted metadata.
- Include tests with a local fake provider and a recovery test that decrypts a backup after restart.
  A cloud-provider test must use a disposable account and synthetic footage.

### Plugins and AI-agent interfaces

- Keep first-party code in Rust and preserve the shared engine command registry. A plugin proposal
  must document its execution boundary, capabilities/permissions, versioning, resource limits, and
  failure isolation; arbitrary native library loading is not accepted by default.
- An MCP adapter should use local stdio transport by default, scope file operations to explicit
  library roots, keep read operations separate from destructive edits/deletion, and never transmit
  footage or credentials to an AI service. Mutations require an explicit user-controlled policy.

### Performance work

- Specify the hardware, resolution, frame rate, duration, workload, baseline, target, and measurement
  method. Bound memory and disk use; do not claim an improvement from an unrepeatable subjective test.

### Maintenance/docs

- State which rule, command, crate, or user-facing setup is corrected and how a fresh checkout
  verifies it. Keep instructions consistent with the actual Rust-only, offline product.

## Definition of done

An issue is complete only when every acceptance checkbox has evidence, relevant tests pass, the
single `cargo xtask ci` gate passes, documentation is updated in the same change, and any
remaining limitation is recorded. UI changes have a screenshot. Capture/export changes include
the platform and failure cases they claim to support. A compile-only result is not completion.

Rank gaps by user impact, frequency, and evidence. `docs/gaps.md` is the ranked work list; update the
entry when evidence, status, estimate, or workaround changes. Do not claim parity percentages until
there is a documented measurement method and reproducible evidence.
