# Upstream ports and technical references

ClipCraft remains a Rust-only product. The ClipCraft maintainer reports having personally obtained
permission from both upstream project authors to port selected source into this repository. This
page records the technical provenance and scope of those ports; it does not replace the permission
terms held by the maintainer. The broader upstream applications, runtimes, interfaces, assets, and
bundled data are not included.

| Upstream | Source revision and file | Rust port | Scope and current use |
| --- | --- | --- | --- |
| [Vice](https://github.com/splitshotz/Vice) | [`vice/main.py` at `7e4b2a3`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/main.py), `_best_game_match` and `_match_game`; [`vice/active_window.py`](https://github.com/splitshotz/Vice/blob/7e4b2a3bccc6e43285d095afe95d0d7fe3d0c84c/vice/active_window.py), `read_steam_app_id` | [`game_detection.rs`](../crates/clipcraft-engine/src/game_detection.rs), [`process.rs`](../crates/clipcraft-platform/src/process.rs), and X11/Windows process-window adapters | Longest matching executable substring wins; exact Steam IDs distinguish shared binaries on Linux; the game list is user-managed and takes no bundled Vice data. The app polls locally every two seconds only while Game-only mode is enabled, and stops the replay buffer when the selected process exits. It does not inspect command lines, process memory, or game files. Upstream project license: GPL-3.0-or-later; the selected source ports rely on the permission reported above. |
| [ClipRecorder](https://github.com/By183/ClipRecorder) | [`src/audio_timeline.h` at `153025b`](https://github.com/By183/ClipRecorder/blob/153025b4c88ed42a23a97e030ce140c2fc54949d/src/audio_timeline.h) | [`audio_timeline.rs`](../crates/clipcraft-record/src/audio_timeline.rs) | Timestamp gap, packet gap, overlap, microphone grace, and recovery policy, with a one-second cap on synthetic loopback silence per poll. Arithmetic is bounded against overflow and the missing-interval start uses `Option`. This utility is for timestamped native audio adapters; the current `AudioSource` mixer does not use native timestamps. The upstream README states that the C++ project is unlicensed; this selected port relies on the permission reported above. |

Each port keeps tests for its behavior. Do not copy unrelated upstream code, bundled game lists,
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
safe Rust X11 and Windows window providers. Steam IDs are read only from the Linux process
environment and only the numeric `SteamAppId`/`SteamGameId` values are retained.
