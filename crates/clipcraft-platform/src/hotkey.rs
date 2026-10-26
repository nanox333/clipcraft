//! Global hotkeys: keys that work while another program, such as a game, has the focus.

#[cfg(not(windows))]
use std::sync::mpsc::Receiver;

/// A shortcut to listen for, as `Ctrl+Alt+Shift+Key` text, with the number the caller wants back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotkeyBinding {
    /// Returned in events so the caller knows which shortcut fired.
    pub id: u32,
    /// The shortcut, such as `F8` or `Ctrl+Shift+R`.
    pub shortcut: String,
}

/// A shortcut was pressed or released.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotkeyEvent {
    /// The [`HotkeyBinding::id`] that fired.
    pub id: u32,
    /// True when pressed, false when released.
    pub pressed: bool,
}

/// Modifiers and key name of a shortcut.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt or Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

/// Splits `Ctrl+Shift+F8` into its modifiers and key name. `None` when there is no key.
pub fn parse_shortcut(text: &str) -> Option<(Modifiers, String)> {
    let mut modifiers = Modifiers::default();
    let mut key = None;
    for part in text.split('+') {
        match part.trim() {
            "Ctrl" => modifiers.ctrl = true,
            "Alt" => modifiers.alt = true,
            "Shift" => modifiers.shift = true,
            "" => return None,
            name => {
                if key.replace(name.to_owned()).is_some() {
                    return None;
                }
            }
        }
    }
    key.map(|key| (modifiers, key))
}

/// Where hotkey events come from.
enum Backend {
    /// A thread forwards events from the display server.
    #[cfg(not(windows))]
    Channel {
        receiver: Receiver<HotkeyEvent>,
        // Dropping this ends the listening thread.
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    },
    /// The operating system delivers events to a hidden window owned by this process.
    #[cfg(windows)]
    Windows(crate::windows::WindowsHotkeys),
}

/// Listens for global hotkeys until dropped.
pub struct HotkeyListener {
    backend: Backend,
    skipped: Vec<String>,
}

impl HotkeyListener {
    /// Starts listening. Shortcuts that cannot be registered (taken by another program, or an
    /// unknown key) are skipped and listed in [`HotkeyListener::skipped`]. On Windows this must
    /// be called on the thread that runs the application's window loop.
    pub fn start(bindings: Vec<HotkeyBinding>) -> Result<Self, clipcraft_capture::CaptureError> {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // On Wayland, X11 grabs only see keys while an X window has the focus, so the
            // keyboards are read instead; when that is not allowed, the grabs are the fallback.
            #[cfg(target_os = "linux")]
            if crate::evdev::wanted()
                && let Ok(listener) = crate::evdev::start(bindings.clone())
            {
                return Ok(listener);
            }
            crate::x11::start_hotkeys(None, bindings)
        }
        #[cfg(windows)]
        {
            crate::windows::start_hotkeys(bindings)
        }
        #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
        {
            let _ = bindings;
            Err(clipcraft_capture::CaptureError::NoProvider)
        }
    }

    /// Like [`HotkeyListener::start`], on an explicit X display. For tests and remote sessions.
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn start_on(
        display: &str,
        bindings: Vec<HotkeyBinding>,
    ) -> Result<Self, clipcraft_capture::CaptureError> {
        crate::x11::start_hotkeys(Some(display), bindings)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) fn from_channel(
        receiver: Receiver<HotkeyEvent>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        skipped: Vec<String>,
    ) -> Self {
        Self {
            backend: Backend::Channel { receiver, stop },
            skipped,
        }
    }

    #[cfg(windows)]
    pub(crate) fn from_windows(
        hotkeys: crate::windows::WindowsHotkeys,
        skipped: Vec<String>,
    ) -> Self {
        Self {
            backend: Backend::Windows(hotkeys),
            skipped,
        }
    }

    /// The next hotkey event, if one has arrived.
    pub fn try_next(&self) -> Option<HotkeyEvent> {
        match &self.backend {
            #[cfg(not(windows))]
            Backend::Channel { receiver, .. } => receiver.try_recv().ok(),
            #[cfg(windows)]
            Backend::Windows(hotkeys) => hotkeys.try_next(),
        }
    }

    /// Shortcuts that could not be registered.
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        match &self.backend {
            #[cfg(not(windows))]
            Backend::Channel { stop, .. } => stop.store(true, std::sync::atomic::Ordering::SeqCst),
            #[cfg(windows)]
            Backend::Windows(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_split_into_modifiers_and_key() {
        let (modifiers, key) = parse_shortcut("Ctrl+Shift+F8").unwrap();
        assert!(modifiers.ctrl && modifiers.shift && !modifiers.alt);
        assert_eq!(key, "F8");
        assert_eq!(parse_shortcut("A").unwrap().1, "A");
        for bad in ["", "Ctrl", "Ctrl+", "A+B", "+A"] {
            assert!(parse_shortcut(bad).is_none(), "{bad:?}");
        }
    }
}
