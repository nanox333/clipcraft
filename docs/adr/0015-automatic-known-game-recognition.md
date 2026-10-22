# ADR 0015 — Automatic recognition of known games

**Status:** accepted · **Date:** 2026-10-22 · **Scope:** M7.1/M7.1a game identity and arming

## Decision

Use the bundled game catalog automatically, following Vice's custom-first detection model.
The user requested Vice-style behavior after reviewing the initial catalog UI. Known games must
not require importing presets before they can be identified and labeled. Keep ClipCraft's existing
game-only contract: no recognized game means no replay capture, and game exit stops capture.

`RecordingSettings::match_game` first checks explicit custom rules, then permitted built-in exact
Steam IDs, then exact executable basenames. Names flow through the existing arming/session and
saved-game metadata path. No window-title guessing, generic launcher match, process injection,
online database query is added. The user subsequently requested all games from Vice; all 347
titles are now compiled as Rust data alongside the existing Bedrock entry, using the maintainer-reported
permission recorded in upstream-ports.md. Existing IDs/variants remain stable; seven launcher/generic
aliases are omitted. Built-ins keep exact matching rather than Vice's substring semantics. Vice's installed-Steam-name fallback and its
compositor/window-class adapters are outside this change.

`detect_built_in_games` defaults to true; `disabled_builtin_games` defaults to empty. Both are
ordinary local settings. The UI exposes the global switch, a searchable known-game list with
per-entry exclusions, and custom rules/overrides. The custom editor is advanced-only: General's
last switch reveals it at the bottom of Games under **Advanced settings**; no separate advanced
category is created. Hiding the editor preserves rule behavior. `recording.set_builtin_games` changes exclusions
atomically for UI and CLI. Existing custom rules keep their matching mode and have precedence,
even over a built-in Steam match or an excluded built-in entry.

## Existing settings and limitations

Settings remain version 1 with optional fields. Older files missing these fields use automatic
recognition on; their existing game-only/manual-mode choice, custom names, rules, and other
preferences are retained. A running recognized game may therefore arm capture after loading an
older game-only configuration. An explicit global-off value or exclusion survives restart and is
never silently reset.

The list contains 348 identification entries; it does not establish live-game, Windows,
anti-cheat, or vendor-wide compatibility. Generic Java/Wine/launcher executables are excluded.
Exclusions and batch operations are bounded at 1,024 IDs, separate from the 64 custom-rule budget;
settings retain the 64 KiB file limit. Process polling remains local every two seconds while game-only mode is enabled; it is skipped
when all built-ins are disabled and there are no custom rules. Idle overhead still needs a
reference-machine measurement. Per-game capture needs the existing platform window provider.

## Verification

Generated tests cover default recognition without custom setup, custom priority over built-in
Steam/executable matches, exclusions/global-off, no-rule idle behavior, start/stop arming,
case/launcher lookalikes, malformed and duplicate exclusions, migration, and persistence.
Keyboard/accessibility and native UI checks exercise the new switches and filtering; isolated CLI
checks cover the same settings command. Run the canonical gate and documentation check.
