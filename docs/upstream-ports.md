# Upstream ports and technical references

ClipCraft remains a Rust-only product. The ClipCraft maintainer reports having personally obtained
permission from both upstream project authors to port selected source into this repository;
the maintainer also requested inclusion of Vice's full game catalog. This
page records the technical provenance and scope of those ports; it does not replace the permission
terms held by the maintainer. The broader upstream applications, runtimes, interfaces, assets, and
other bundled data are not included. Vice's identification catalog is the explicit data exception
documented below.

| Upstream | Source revision and file | Rust port | Scope and current use |
| --- | --- | --- | --- |
| [Vice](https://github.com/splitshotz/Vice) | [`vice/main.py` at `7e4b2a3`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/main.py), `_best_game_match` and `_match_game`; [`vice/active_window.py`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/active_window.py), `read_steam_app_id` | [`game_detection.rs`](../crates/clipcraft-engine/src/game_detection.rs), [`process.rs`](../crates/clipcraft-platform/src/process.rs), and X11/Windows process-window adapters | Longest matching executable substring wins; exact Steam IDs distinguish shared binaries on Linux; custom rules are user-managed and override the automatic catalog adapted from Vice's bundled identification data. A bounded worker polls locally every two seconds while automatic switching is enabled. Games-only stops when the process exits; opt-in policies switch to timed or continuous desktop replay. It does not inspect command lines, process memory, or game files. Upstream project license: GPL-3.0-or-later; the selected source ports rely on the permission reported above. |
| [ClipRecorder](https://github.com/By183/ClipRecorder) | [`src/audio_timeline.h` at `153025b`](https://github.com/By183/ClipRecorder/blob/153025b4c88ed42a23a97e030ce140c2fc54949d/src/audio_timeline.h) | [Historical Rust utility at `62bed30`](https://github.com/nanox333/clipcraft/blob/62bed30/crates/clipcraft-record/src/audio_timeline.rs) | Timestamp gap, packet gap, overlap, microphone grace, and recovery policy, with a one-second cap on synthetic loopback silence per poll. Arithmetic is bounded against overflow and the missing-interval start uses `Option`. Removed in M11.1j because no production caller used it; no ClipRecorder source remains in the current tree. The active `AudioSource` mixer is unchanged. The upstream README states that the C++ project is unlicensed; this selected port relies on the permission reported above. |

Current ports keep tests for their behavior. The removed ClipRecorder utility and its tests remain
in Git history; the table preserves provenance rather than claiming current reuse. Do not copy unrelated upstream code, additional bundled data,
artwork, or runtime components. Keep these source paths and revisions current if a port changes.

## GPU Screen Recorder as a capture reference

[GPU Screen Recorder's README at `1c01417`](https://git.dec05eba.com/gpu-screen-recorder/plain/README.md?id=1c014172dd3e89732bc09d5ca34224be7ccff02a)
describes capture on X11 and Wayland with NVIDIA, AMD, and Intel hardware, using vendor capture and
encode paths such as NVFBC/NVENC and VA-API. Its own limitations matter: AMD/Intel monitor capture
through KMS requires elevated access, while window and portal capture do not; portal/compositor and
driver behavior can affect frame rate, HDR, and capture availability. This is a technical behavior
reference for the proposed Rust-native M5.2 path, not a claim of universal compositor compatibility.

No GPU Screen Recorder source or binary is ported or shipped. The project is GPL-3.0-only and uses a
non-Rust stack including FFmpeg; shipping or translating its source would conflict with ClipCraft's
current permissive-dependency policy. ClipCraft may independently implement equivalent capture
behavior in Rust against public platform APIs. Keep the CPU-frame fallback, avoid privileged KMS
helpers, and measure CPU/GPU/game-frame impact before making performance claims.

Process enumeration uses [`sysinfo` 0.39.6](https://github.com/GuillaumeGomez/sysinfo), an MIT-licensed
Rust dependency. Vice's Python window/compositor adapters were not copied; ClipCraft uses its own
safe Rust X11 and Windows window providers. Linux process environments supply only numeric `SteamAppId`/`SteamGameId` values. Verified
local installation paths can also supply exact Steam IDs; full process paths are not retained.

## Bundled game identification metadata

All 347 titles from [`vice/data/games.json` at `7e4b2a3`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/data/games.json)
are adapted into [`game_catalog_data.rs`](../crates/clipcraft-engine/src/game_catalog_data.rs).
The pinned 33,596-byte JSON source has SHA-256
`c4ac15ea4388837c5a36132dbaa1ae6186206c60d6b4301a7e3a0c98f9b7b746`.
This inclusion relies on the maintainer-reported author permission above, rather than asserting
that Vice's GPL-3.0-or-later project license is a permissive license. Upstream provenance remains
attached to the converted data; this record does not invent or replace the author's permission terms.

Adaptations: all upstream titles are retained; `steam_app_<id>` becomes an exact numeric Steam ID;
process aliases are deduplicated case-insensitively; the original ClipCraft IDs and identification
variants are retained; Minecraft Bedrock remains an extra entry. Seven launcher/generic aliases
are omitted as listed in [game-catalog.md](game-catalog.md). No upstream application runtime, art,
icons or game footage is included. Compiled Rust data needs no parser, service or download.

The default/custom precedence was rechecked against Vice `7e4b2a3` `_match_game`: custom definitions
precede the automatically loaded bundled list. ClipCraft follows that model with per-game exclusions
and a global switch (ADR 0015), but keeps exact built-in basenames instead of substring defaults.
An original bounded Rust adapter now supplies local installed-Steam names/IDs (ADR 0016);
no additional Vice source or database was imported for it. Window-class adapters remain absent. Tests check all
348 resulting entries, 672 process aliases and 311 Steam IDs; source/title parity is also compared
against the pinned upstream JSON during verification. These are matching checks, not live-game proof.

## Larger Steam metadata reference

[leinstay/steamdb](https://github.com/leinstay/steamdb) was inspected at
`6ade3eeaf15d6ac3de2de32cb92eb59c71b6f4bd`. Its README describes 194,282 rows with Steam IDs,
titles and third-party store metadata, but no executable mapping. It declares GPL-3.0; no data or
code from it is imported and no runtime request is made. The permission for Vice/ClipRecorder is
not treated as permission for this repository. Local installed-Steam-manifest recognition is a
separate implemented Rust path described in [game-catalog.md](game-catalog.md).
