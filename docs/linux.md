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
| Global hotkeys | key grabs | keyboards read through evdev | X11: virtual display and `xdotool`. evdev: byte-level tests and a fake device file; not on a real keyboard |
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
  * For global hotkeys: read access to the keyboards (next section).

## Hotkeys on Wayland

A Wayland program cannot grab a key for the whole desktop, so ClipCraft reads the keyboards
(`/dev/input/event*`). That needs permission, and it is a real privacy trade-off: whatever can read
the keyboard can read what you type. Pick one:

1. **Join the `input` group:** `sudo usermod -aG input $USER`, then sign out and in. (Every program
   you run can read the keyboards from then on.)
2. **Install the udev rule** in `packaging/linux/70-clipcraft-keyboard.rules`, which grants the person
   signed in at the computer the same access. The file explains how to install and remove it.
3. **Neither:** ClipCraft then only sees keys pressed while an X window (a game running under
   XWayland) has the focus, and falls back to X11 grabs. Bind a desktop shortcut to your own
   command instead; in KDE, GNOME and most others that is Settings > Keyboard > Shortcuts.

ClipCraft never records or stores keys; it only matches the shortcuts you set. Shortcut letters use
the physical (US) key positions, so on other layouts a letter shortcut is where the US key is.
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
* The xdg-desktop-portal GlobalShortcuts interface (KDE, GNOME 48+, Hyprland), which would avoid
  reading the keyboards at all.
