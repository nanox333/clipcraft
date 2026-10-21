# Release checklist

What must be true before a build is called a beta, and the evidence for each row as of today.
"Verified here" means a test or run in the Linux build container (a virtual X display, generated
sources, a null PulseAudio sink, a tmpfs for full-disk cases, ffprobe/ffmpeg as outside judges).
"Open" rows need real machines or the maintainers' signing keys. No row is ticked from a build
alone.

| Gate | State | Evidence or what is missing |
| --- | --- | --- |
| Works with no network | Verified here | An `strace` of the app starting, clipping, and saving opens only `AF_UNIX` sockets (X11, sound server); no Cargo dependency is a network client. |
| Crash recovery | Verified here | Unfinished saves cleaned up or recovered on launch, MP4 crash-safe copy, damaged settings/index set aside ([ADR 0011](adr/0011-crash-and-disk-safety.md)). Open: a real kill during a write. |
| Full disk | Verified here | Real tmpfs tests: refusal before writing, mid-save and mid-export failure leave nothing, earlier clips untouched. |
| Device loss | Verified here (generated) | A lost microphone goes silent and rejoins; a lost screen keeps what was recorded. Open: a real unplugged device. |
| Suspend/resume | Verified here (simulated) | The soak tool freezes the recorder with `SIGSTOP` for 20 s; recording resumes, the stale buffer is dropped after a silence of 10 s, and later clips have no frozen stretch. Open: a real laptop sleep. |
| Resource budget | Verified here (synthetic) | `cargo run --release -p clipcraft-record --example soak -- --minutes N`. 720p30 with sound on a shared 4-core container: memory +21 to +26 MB after warm-up (budget 64 MB) over 3 to 4 minutes, 1.5 to 2.7 % pictures dropped under load, picture and sound within 20 ms in every clip. Open: the two-hour run, and GPU and power figures on reference hardware. |
| Keyboard access | Verified here | Tab reaches every control; the focused control gets a visible ring; Enter activates it (checked in the running app). |
| Screen reader names | Verified here | A test walks the accessibility tree of the library, clip view, and every settings tab and fails on any unnamed control. Open: an actual screen reader (NVDA, Orca, VoiceOver). |
| Diagnostics | Verified here | `clipcraft-cli diagnose [--json]` and Settings › General › Copy report. |
| Installer | Partly | `cargo xtask package` builds a Linux tarball (18 MB; binaries, desktop entry, licences, and `SHA256SUMS`). Run to completion on Linux x86_64: the checksum verifies, the archive unpacks into an empty folder, every shared library the binaries need is present, and `clipcraft-cli diagnose` runs from it. It was not installed on a clean second machine. Open: Windows installer, macOS bundle, code signing and notarisation (need the maintainers' certificates), an update story (there is none by design). |
| Windows capture | Partial | Windows/AMD generated-window WGC-to-MP4 runs and real D3D11 lease/color/resize tests pass. Hotkeys, games, device loss and sustained 1080p60 remain open; see [measurements](windows-performance.md). |
| Hardware encoders | Partial | AMD Media Foundation has generated-window CPU/native-surface evidence. NVENC, VideoToolbox and VA-API real-device claims remain open. |
| Real sound devices | Open | Microphone and speaker capture never run with real hardware. |

## How to repeat the evidence

- `cargo xtask ci` runs every automated check (format, strict lints, all tests, layers, assets,
  licences). Disk-full tests need permission to mount a tmpfs and print "skipped" otherwise.
- Soak: `cargo run --release -p clipcraft-record --example soak -- --minutes 120 --freeze-for 30`
  (needs ffprobe). It exits non-zero when a budget is broken.
- Offline: `strace -f -e trace=network -o log clipcraft`, then look for `AF_INET`.
- Diagnostics: `clipcraft-cli diagnose`.

## Publishing a release on GitHub

`.github/workflows/release.yml` builds the Linux (x86_64) tarball and the Windows (x86_64) zip with
`cargo xtask package`, writes `SHA256SUMS.txt`, and creates a **draft** GitHub Release for review.

1. Set the version in `Cargo.toml` (`[workspace.package]`) and merge it.
2. Push a tag that matches it, for example `git tag v0.1.0 && git push origin v0.1.0` (or run the
   workflow by hand from the Actions tab).
3. Open the draft release, check the files, and publish it.

The downloads are **not code-signed**, so Windows SmartScreen shows "unknown publisher" until a user
chooses More info, then Run anyway; `INSTALL.txt` in each download says so. The workflow has **not
been run yet**: the Linux package is verified locally, the Windows build is only known to compile
and pass the quality gate on Windows CI, and neither the Windows zip nor the release step has run.
There is no macOS download (no macOS provider yet).
