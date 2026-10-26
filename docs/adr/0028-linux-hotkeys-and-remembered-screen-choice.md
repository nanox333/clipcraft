# ADR 0028: Linux global hotkeys by reading keyboards, and a remembered screen choice

Status: accepted, 2026-10-26.

## Context

X11 key grabs (what ClipCraft used on every Linux session) only see a key while an X window has the
focus. On Wayland, where games usually run under XWayland and the desktop itself is Wayland-native,
that means the save-clip key often does nothing. Wayland deliberately offers no way to grab a key for
the whole desktop; the options are the GlobalShortcuts portal (KDE, GNOME 48+, Hyprland only), a
desktop-specific shortcut bound to a command, or reading the keyboard devices. The ScreenCast portal
also shows its chooser on every start unless the program passes back the `restore_token` it was given.

## Decision

* On a Wayland session, global hotkeys read `/dev/input/event*` keyboards (found through
  `/proc/bus/input/devices`) with plain `poll` and `read`: no `unsafe`, nothing grabbed or swallowed.
  If no keyboard can be read, the X11 grabs are used as before.
* Access is the person's choice: the `input` group or an optional udev rule shipped in
  `packaging/linux/`. ClipCraft installs nothing and documents the privacy cost.
* The ScreenCast portal is asked to persist the choice (`persist_mode` 2, portal version 4 and newer)
  and the returned `restore_token` is kept in the ClipCraft data folder, readable by this user only.
  Deleting the file makes the desktop ask again.
* Windows and X11 behaviour is unchanged.

## Consequences

Works under every compositor once keyboards are readable; letter shortcuts follow physical (US) key
positions. Not done: the GlobalShortcuts portal, which avoids keyboard access altogether and is the
better route where available. The decision is informed by public documentation of other Linux
clippers; no code was taken from them.
