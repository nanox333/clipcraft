# ADR 0032: Optional update check and installer hand-off

Accepted 2026-10-28.

ClipCraft's core workflows stay offline. The one network use is an optional update check: a single
HTTPS `GET` of GitHub's "latest release" description for this project, made once at start-up when
Settings → General → **Check for updates** is on (the default) or when the person presses **Check for
updates** in About. The request carries no identifiers beyond a `User-Agent: ClipCraft`. Turn the
setting off and ClipCraft never goes online.

Nothing is downloaded until the person presses **Update now**. The engine then fetches the Windows
setup program and `SHA256SUMS.txt` from this project's releases only, refuses a setup program whose
SHA-256 does not match the published list, writes it to a temporary folder, and the app starts it
and closes. The setup program upgrades in place and keeps the earlier version directory (ADR 0031).
Checksums protect against corruption and a tampered download, not against a compromised release;
code signing remains open.

Layering: `clipcraft-engine::update` is pure (version comparison, release parsing, URL and file-name
checks, checksum verification) and registers an HTTP fetcher at start-up, like the Recycle Bin
handler; `clipcraft-platform::https_get` is the fetcher. It uses the operating system's TLS through
the already-present `native-tls`, follows redirects only between GitHub hosts, and bounds every
response. It contains no `unsafe`. Linux has no system TLS here, so update commands report that
updates are unavailable. Commands `app.check_for_update` and `app.download_update` are shared by
the app and `clipcraft-cli update check`. `sha2` 0.10 (MIT OR Apache-2.0, pure Rust) is added for
the checksum.

Drafts and pre-releases are never offered. A release without a `ClipCraft-Setup-*.exe` and a
`SHA256SUMS.txt` is not an update.
