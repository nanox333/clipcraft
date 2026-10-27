# ADR 0031: Rust offline Windows installer

Accepted 2026-10-27 for M5.1a. The user requested an installer, first-run setup and multiple shortcuts.

The existing portable archive remains available. A native egui `clipcraft-installer` workspace
binary carries a gzip payload appended by Rust xtask. No NSIS, WiX, scripting runtime, hosted
bootstrapper or download is required. `flate2` 1.1.10 is already present in Cargo.lock; direct reuse
pins that reviewed source (MIT OR Apache-2.0) with `rust_backend`, without a C backend. This crate
uses only safe streaming Read/Write APIs; decoded bytes, file counts, paths and version history
are capped. Gzip checksums detect corruption; release SHA256SUMS are still required. This is not
code signing or authenticity verification.

Engine commands `app.install` / `app.uninstall` write only a fixed set of program/notices files.
Installations use new version directories and publish their bounded ownership record last.
An unrelated nonempty destination, symlink/junction ancestry, malformed payload or ownership
record is rejected. Upgrades retain earlier executable directories, allowing a running replay
to finish. At eight versions the user must uninstall/reinstall; no automatic updater is added.
Removal enumerates owned files, never recursively deletes a caller-supplied directory, and keeps
unknown files, clips, settings and library metadata. Installation does not enable login startup.

Windows integration is in `clipcraft-platform`. IShellLinkW and IPersistFile are called through
the existing windows 0.62 bindings. Every unsafe block has a SAFETY comment. The worker initializes
and balances its COM apartment, owns typed COM references, and keeps checked, absolute,
NUL-terminated UTF-16 buffers alive throughout each synchronous call. The fallback is actionable
failure with the installed executable left available. HKCU uninstall registration uses the OS
`reg.exe` command with separate arguments and a hidden console, matching existing startup support.
Registry reads use bounded UTF-16 RegGetValueW calls so Unicode installation folders do not depend
on the console code page. Shell known folders honor redirected Desktop and Start menu locations;
their OS-allocated buffers are converted then freed exactly once with CoTaskMemFree. Existing
ClipCraft login entries pointing inside this install root migrate on upgrade and are removed on
uninstall; installation never enables login startup for a user who did not have it enabled.
No administrator privileges or required non-Rust build tools are introduced.

Windows cannot remove its running uninstaller. The installed stub launches a uniquely created
copy in the OS temporary directory; that temporary program is left for normal temporary-file
cleanup. Native UI requires explicit confirmation before removal. The unattended, explicit
`--install-to <absolute-folder> --accept-license --no-integration` and
`--remove-from <absolute-folder> --no-integration` paths call the same engine commands and allow
isolated install/upgrade/remove evidence without changing the user's Start menu or registry.

Onboarding edits a draft and saves through `recording.save_settings`. No capture, microphone or
Discord worker starts during setup. Existing settings without a completion field are treated as
already configured; fresh defaults request setup. Closing before completion leaves setup pending.
Hotkeys retain legacy primary strings, with at most four extra bindings in action order; invalid
and duplicate values are bounded and sanitized. Registration IDs distinguish held PTT alternatives.

Windows signing, clean-machine/SmartScreen testing, macOS installers and delta updates remain open.
