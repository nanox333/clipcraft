# ADR 0012: Wayland capture through the ScreenCast portal and PipeWire

- **Status:** Accepted. The portal conversation and the PipeWire receiver are verified here against a
  stand-in desktop and a real PipeWire server. A real desktop's chooser is **not** verified (see
  [docs/wayland.md](../wayland.md)).
- **Date:** 2026-10-20

## Context

A Wayland program cannot read the screen. The desktop shows its own chooser (the
`org.freedesktop.portal.ScreenCast` portal) and hands back a PipeWire node. X11 capture on a Wayland
desktop only sees XWayland windows, so it must not be offered there.

## Decision

- **Portal over D-Bus with `zbus`** (pure Rust, blocking API, no `unsafe`). The handshake is
  CreateSession, SelectSources (monitor or window, cursor drawn into the picture when the desktop
  allows it), Start, OpenPipeWireRemote; each answers through a `Response` signal that is subscribed
  to *before* the call. The chooser may stay open for three minutes.
- **PipeWire through `libpipewire-0.3` loaded at run time** (the same approach as VA-API and
  PulseAudio): a computer without it reports so. The only `unsafe` is `wayland/pipewire.rs`, with
  `// SAFETY:` comments, callbacks wrapped in `catch_unwind`, and structure layouts asserted against the
  C headers. The format offer is built by hand and compared byte for byte with PipeWire's own builder.
  Only shared-memory buffers are accepted (no GPU buffers), so every desktop sends something we can read.
- **One target:** on a Wayland session `list_targets` offers a single entry, "Screen or window (your
  desktop will ask)"; the real name and size are known once the person has chosen.
- **The chooser never blocks the window.** The app opens the share on a worker thread and shows a
  notice while the desktop's dialog is open.
- **A share that ends is an error, not a hang.** PipeWire pauses a stream when its node goes away; a
  stream that was sending and has been paused for 5 s is reported as the screen having gone away.
  A still screen that sends nothing repeats its last picture.

## Consequences

- Each launch (or each start) asks the person again; remembering the choice (portal restore tokens)
  is not built.
- Window capture, "force window capture", and "the window in front" do not exist on Wayland: the
  desktop's chooser decides, and programs cannot learn which window has the focus.
- GPU (DMA-BUF) buffers are not used, so very large screens cost a copy per picture.
- New dependency: `zbus` (MIT) and its pure-Rust tree.
