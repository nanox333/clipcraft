# Wayland capture

How ClipCraft captures a Wayland screen, how it is tested here, and what has not been seen on a
real desktop. Decision record: [ADR 0012](adr/0012-wayland-capture.md).

## How it works

1. `clipcraft-platform` treats the session as Wayland when `WAYLAND_DISPLAY` is set, `libpipewire-0.3`
   loads, and `org.freedesktop.portal.ScreenCast` answers on the session bus. Otherwise X11 is used.
2. Starting a clip opens the portal session. The desktop shows its chooser; ClipCraft shows
   "Choose the screen or window to clip in the window your desktop shows".
3. The portal returns a PipeWire node and a connection to the PipeWire server that owns it.
   ClipCraft connects a stream to it, offers raw video (BGRx, BGRA, RGBx, RGBA, any size and rate,
   shared memory only), answers a format that still has choices with its defaults, and copies each
   picture into a bounded BGRA frame.
4. Stopping closes the stream, then the portal session.

## What was tested

Global shortcuts use a separate GlobalShortcuts portal session. A private D-Bus fake covers
registration, assignment changes, press/release, denial, cancellation, overflow, service loss and
retry. Direct keyboard-device access is a separate default-off fallback; see [Linux](linux.md).
Real desktop shortcut dialogs and physical keyboards have not been tested.
The shortcut bind request currently uses an empty parent-window identifier, so compositor-specific
dialog placement and focus are also unverified.

| Layer | How | Result |
| --- | --- | --- |
| Format offer | Byte comparison with the pod PipeWire's C builder writes (`wayland/pod.rs`) | exact match |
| Reading a format | Cut-off and bit-flipped copies of an agreed format, plus the choice-bearing form GStreamer's PipeWire sink returns | never panics; defaults read |
| C structure layouts | Offsets and sizes asserted against `pipewire/*.h` measured with gcc (`wayland/pipewire.rs`) | match |
| PipeWire receiver | Real PipeWire 1.0.5 server; GStreamer `videotestsrc` published as a video node in BGRx and RGBx; pictures compared for colour | correct colour and size, 5+ fresh pictures in 10, stops in under 5 s |
| Node disappears | The producer is killed mid-share | reported as the screen going away within about 5 s |
| Portal conversation | A fake `org.freedesktop.portal.ScreenCast` on a private session bus, answering with `Response` signals like the real one, handing out a real PipeWire connection | order of calls, cursor mode, colours, session closed on stop, a declined chooser is `Denied`, no portal is an error |

Run them with a PipeWire server, `pw-cli`, `gst-launch-1.0` (with the PipeWire plugin) and
`dbus-daemon`; each test says "skipped" and why when one is missing.

## What was not verified

- **A real desktop's chooser.** `xdg-desktop-portal-wlr` 0.7.1 on headless sway 1.9 (pixman renderer)
  answers the first three calls and then declines to start ("unable to receive a valid format from
  wlr_screencopy"), although `grim` captures the same compositor. That is the portal backend, not
  ClipCraft, but it means **no real portal backend has delivered a picture to ClipCraft.** GNOME,
  KDE, and a GPU-backed wlroots desktop are untested, so are DMA-BUF-only compositors, fractional
  scaling, and several monitors.
- Window sharing, the cursor option, and what each desktop does when the person stops the share
  from its own indicator.
- Restoring a previous choice so the chooser does not appear every time.

## Setting up the test desktop

```
apt install sway pipewire wireplumber xdg-desktop-portal xdg-desktop-portal-wlr \
    gstreamer1.0-pipewire gstreamer1.0-tools libpipewire-0.3-dev grim dbus-daemon
```

Start a private session bus, `sway` with `WLR_BACKENDS=headless WLR_RENDERER=pixman`, `pipewire`,
`wireplumber`, `xdg-desktop-portal-wlr` (with `chooser_type=none` in its config) and
`xdg-desktop-portal` on that bus; export `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS` and
`WAYLAND_DISPLAY`; then `cargo test -p clipcraft-platform --test wayland --test wayland_portal`
and `cargo test -p clipcraft-platform --lib wayland`.
