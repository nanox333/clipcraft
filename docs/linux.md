# ClipCraft on Linux

What works on which kind of Linux computer, what each part needs, and how to check. Run
`clipcraft-cli diagnose` (the app has the same report) to see this for your own computer.

Approach to Wayland hotkeys and remembered screen choice was informed by the public documentation of
other Linux clippers (for example Vice's README: global keys read from the keyboards, `input` group
or a udev rule). No code from them was used or ported; they are GPL-licensed.

## What works where

| Part | X11 | Wayland | Verified how |
| --- | --- | --- | --- |
| Screen capture | core protocol and shared memory (MIT-SHM) | desktop's screen-cast portal and PipeWire | X11: virtual display, real clips decoded by ffmpeg. Wayland: a fake portal against a real PipeWire server; **no real GNOME, KDE or wlroots desktop has delivered a picture yet** |
| Global hotkeys | key grabs | GlobalShortcuts portal; optional evdev or limited XWayland fallback | X11: virtual display. Portal: private D-Bus fake. evdev: synthetic device file; no real Wayland desktop or physical keyboard |
| Game recognition | process list (Steam IDs, names) | process list | virtual display; no real game |
| Sound | PulseAudio client library (also answered by PipeWire) | same | a PulseAudio null sink |
| Encoding | processor (pure Rust H.264); NVIDIA NVENC and AMD/Intel VA-API when present | same | processor only; no graphics card was available |

## What the computer needs

* A 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36, Arch and anything
  newer). Older systems (Ubuntu 20.04, Debian 11, RHEL 8) cannot run the download.
* `libasound.so.2` (ALSA), which nearly every desktop already has (`libasound2` on Debian and Ubuntu,
  `alsa-lib` on Fedora and Arch). The program does not start without it.
* An OpenGL or Vulkan driver (Mesa or the vendor's) for the window.
* **X11 session:** nothing else.
* **Wayland session:**
  * PipeWire and `xdg-desktop-portal` with your desktop's backend (`xdg-desktop-portal-gnome`,
    `-kde`, `-wlr` or `-hyprland`). The desktop shows its own screen chooser the first time; ClipCraft
    remembers the choice (portal version 4 and newer) so it does not ask again. To be asked again,
    delete `screencast-restore-token` in the ClipCraft data folder (`~/.local/share/ClipCraft`).
  * For global hotkeys: a desktop backend implementing GlobalShortcuts; otherwise see the fallbacks
    below. Direct keyboard access is not enabled by default.

## Hotkeys on Wayland

A Wayland program cannot grab a key for the whole desktop directly. ClipCraft first asks the
desktop's GlobalShortcuts portal to register its shortcuts. The desktop may display a consent or
configuration dialog, choose different keys, or decline some bindings. Settings › Hotkeys displays
the desktop's assignments, reports failures and offers **Retry desktop shortcuts**. Support varies
by desktop and has not yet been verified against a real compositor.
If shortcut events are interrupted, ClipCraft closes push-to-talk until its key is released and
pressed again, so a missed release cannot leave the microphone open.

If the portal is unavailable or declined, ClipCraft tries X11 grabs, which only see keys while an
XWayland window has focus. The person's configured local shortcuts also work in the focused
ClipCraft window when not registered with the portal. Push-to-talk requires a working global
listener. Native Wayland games may not deliver shortcuts through the X11 fallback.

Alternatively, explicitly turn on **Allow direct keyboard access if desktop shortcuts fail** in
Settings › Hotkeys. The fallback reads `/dev/input/event*` and still needs operating-system
permission. This is a real privacy trade-off: other programs with the same access can also read
what you type. To grant permission, choose one:

1. **Join the `input` group:** `sudo usermod -aG input $USER`, then sign out and in. (Every program
   you run can read the keyboards from then on.)
2. **Install the udev rule** in `packaging/linux/70-clipcraft-keyboard.rules`, which grants the person
   signed in at the computer the same access. The file explains how to install and remove it.
3. **Neither:** keep direct keyboard access disabled and use the limited XWayland/focused-window
   fallback described above.

ClipCraft never records or stores raw keys; it only matches the shortcuts you set. With the direct
keyboard fallback, shortcut letters use physical US key positions; portal key interpretation is
controlled by the desktop.
Unplugging and re-plugging a keyboard is noticed within a few seconds.

## If something does not work

* `clipcraft-cli diagnose` lists the session type, hotkey access, Wayland portal, monitors and
  encoders, each with what to install.
* "Waiting for game" stays on: game-only capture is on and no recognised game is running. Choose
  Desktop capture in Settings > Capture, or add the program as a custom game.
* "Choose the game window to record": pick a source in Settings > Capture or switch to Desktop.
* The clip drops frames ("dropping 55% of frames"): the processor cannot keep up at that size. Lower
  the resolution or frame rate in Settings > Quality, or use a graphics card encoder. On a 4-core
  machine the pure-Rust encoder reaches roughly 50 pictures per second at 1440x900.
* Running ClipCraft from a systemd user service or a launcher on a Wayland desktop: the service must
  inherit `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` from your session (most
  compositors need a line in their config that imports them into systemd's environment).

## Not done yet

* No real Wayland desktop (GNOME, KDE, Hyprland, sway, Niri) has been tried; DMA-BUF-only
  compositors, fractional scaling and several monitors are untested.
* Tagging a clip with the game in front on Wayland: Wayland does not say which window has the focus.
* Flatpak and other sandboxed packages (including immutable Fedora variants); AppImage.
* Real GlobalShortcuts portal compatibility and consent behavior on GNOME, KDE and wlroots desktops.
