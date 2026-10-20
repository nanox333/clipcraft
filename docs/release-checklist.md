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
| Windows capture | Open | Compiles; never run on Windows. |
| Hardware encoders | Open | NVENC, VideoToolbox, Media Foundation, and VA-API code is tested against stand-ins only. |
| Real sound devices | Open | Microphone and speaker capture never run with real hardware. |

## How to repeat the evidence

- `cargo xtask ci` runs every automated check (format, strict lints, all tests, layers, assets,
  licences). Disk-full tests need permission to mount a tmpfs and print "skipped" otherwise.
- Soak: `cargo run --release -p clipcraft-record --example soak -- --minutes 120 --freeze-for 30`
  (needs ffprobe). It exits non-zero when a budget is broken.
- Offline: `strace -f -e trace=network -o log clipcraft`, then look for `AF_INET`.
- Diagnostics: `clipcraft-cli diagnose`.
