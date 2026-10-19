# ClipCraft — rules for agents and contributors

These rules apply to every human and AI contributor. `CLAUDE.md` describes the working process;
this file contains the rules that must not be broken. If the two disagree, this file wins.

## 0. Rust only; no hosted service

- **The product is written in Rust.** All first-party application code, UI, engine, CLI, local
  library metadata, and required build/test tools belong in the Rust Cargo workspace. Do not add JavaScript,
  TypeScript, Node, Python, C, or C++ as a product runtime, sidecar, UI, or required build step.
- Use a native Rust desktop UI (initially egui/eframe). Do not add Electron, Tauri, a browser app,
  or a web frontend.
- **ClipCraft must not require a hosted server or a server process.** Install, launch, capture,
  manage clips, edit, and export offline. Do not add a backend, account, upload, cloud sync,
  share-link service, telemetry endpoint, or network-dependent feature. Users can share exported
  files with software they choose.
- Pure Rust still needs OS media APIs. Put platform bindings behind `clipcraft-platform` and a safe
  `Result`-returning interface. `unsafe` is forbidden everywhere else; any platform `unsafe` needs
  an ADR, a `// SAFETY:` comment for every block, and a tested fallback where practical.
- Use permissively licensed dependencies. The one exception is MPL-2.0 for the unmodified `symphonia*`
  audio decoders pulled in by FilmCraft ([ADR 0005](docs/adr/0005-mpl-symphonia-exception.md)). Implement formats from public specifications. FFmpeg and
  ffprobe may generate test fixtures or serve as external test oracles; never link, bundle, or ship
  them.

## 1. Never crash or lose a clip

Malformed media/library metadata, invalid commands, device loss, a full disk, or a failed export must return
an actionable error and preserve existing files. This rule outranks feature work.

- Return `Result<T, E>` and propagate errors with `?`; add context instead of discarding errors.
- No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, or `unimplemented!` in non-test
  code. An unfinished operation returns an explicit unsupported error.
- Treat every input-derived number, path, duration, stream, and allocation size as hostile. Check
  indices and arithmetic, guard division and casts, cap allocations, and bound recursion and loops.
- Handle poisoned locks and failed worker joins. Background jobs must report errors instead of
  leaving the UI in a permanent running state.
- Crash recovery is a safety net, not permission to add panic paths. Every crash fix needs a focused
  regression test that fails before the fix.

## 2. Local media and privacy

- Captures and library metadata remain on the user's device. Never upload, sync, or transmit media.
- Do not overwrite source media during export. Use atomic writes, collision checks, and explicit
  confirmation before destructive deletion.
- Do not log captured paths, audio content, raw environment values, or private library metadata.
- Keep user media and reference screenshots out of Git. Generate deterministic fixtures or fetch
  pinned, hash-verified test corpora into ignored build/cache directories.

## 3. Clean-room and assets

- Observe Medal's public product behavior and supplied screenshots for layout and interaction only.
  Do not copy Medal code, logos, icons, game footage, screenshots into the product, text, private
  APIs, or network protocol. Use original ClipCraft artwork and generated or openly licensed media.
- Every shipped image, icon, font, sound, video, and other non-code asset needs a permissive license
  and an attribution entry in `ATTRIBUTION.md`. Unknown licenses are not accepted.
- Never copy code or assets from proprietary products. Read public format specifications, not
  proprietary application internals.

## 4. Architecture and behavior

- Keep capture, timing, media, library, edit, render, export, platform, and UI responsibilities in
  the layers described by [`docs/architecture.md`](docs/architecture.md). Lower layers must not
  depend on egui or platform APIs.
- Use integer `Tick` time and rational frame rates for edit math; do not use floating-point seconds
  for clip boundaries or synchronization.
- Every user operation is an engine command with a stable ID, typed parameters, validation, and
  tests. The UI and CLI call the same command implementation. The CLI uses local files/stdin/stdout;
  do not add a control server.
- Keep replay buffers, frame queues, disk use, and background work explicitly bounded.

## 5. Tests and completion

- A change is not done until its issue acceptance criteria pass, tests cover the behavior and failure
  cases, docs and [`docs/gaps.md`](docs/gaps.md) reflect any remaining limitations, and
  `cargo xtask ci` passes.
- Parser, media, timing, and command changes require malformed/hostile-input tests. Crash fixes need
  a regression test. UI changes require a screenshot at a representative desktop size and keyboard
  checks. Capture changes require deterministic fake-provider tests plus platform integration proof.
- Do not claim a milestone or parity percentage without measured evidence.
