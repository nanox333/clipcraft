//! Global hotkeys: keys that work while another program, such as a game, has the focus.

#[cfg(not(windows))]
use std::sync::mpsc::Receiver;

/// How global shortcut registration is progressing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HotkeyStatus {
    /// The desktop is being asked for permission.
    Pending,
    /// The desktop portal is active; these configured shortcuts were not bound.
    Portal {
        unbound: Vec<String>,
        assigned: Vec<(String, String)>,
    },
    /// The portal was unavailable or closed and a fallback is active.
    Fallback {
        direct_keyboard: bool,
        denied: bool,
        unbound: Vec<String>,
    },
    /// Global shortcuts are unavailable; in-window handling remains available.
    Unavailable { denied: bool },
    /// A non-portal listener is active.
    Ready,
}

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
    // The plus key itself is written `Ctrl++`: a trailing `+` after a separator.
    let (text, plus) = match text.strip_suffix("++") {
        Some(rest) => (rest, true),
        None if text == "+" => ("", true),
        None => (text, false),
    };
    if plus {
        key = Some("+".to_owned());
    }
    for part in text.split('+').filter(|_| !(plus && text.is_empty())) {
        match part.trim() {
            name if name.eq_ignore_ascii_case("ctrl") => modifiers.ctrl = true,
            name if name.eq_ignore_ascii_case("alt") => modifiers.alt = true,
            name if name.eq_ignore_ascii_case("shift") => modifiers.shift = true,
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
        worker: std::thread::JoinHandle<()>,
    },
    /// The operating system delivers events to a hidden window owned by this process.
    #[cfg(windows)]
    Windows(crate::windows::WindowsHotkeys),
}

/// Listens for global hotkeys until dropped.
pub struct HotkeyListener {
    backend: Backend,
    skipped: Vec<String>,
    #[cfg(target_os = "linux")]
    pub(crate) portal_status: Option<std::sync::Arc<std::sync::Mutex<HotkeyStatus>>>,
    #[cfg(target_os = "linux")]
    pub(crate) overflow: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl HotkeyListener {
    /// Starts listening. Shortcuts that cannot be registered (taken by another program, or an
    /// unknown key) are skipped and listed in [`HotkeyListener::skipped`]. On Windows this must
    /// be called on the thread that runs the application's window loop.
    pub fn start(bindings: Vec<HotkeyBinding>) -> Result<Self, clipcraft_capture::CaptureError> {
        Self::start_with_options(bindings, false)
    }

    /// Start global shortcuts, allowing raw keyboard access only when explicitly selected.
    pub fn start_with_options(
        bindings: Vec<HotkeyBinding>,
        allow_raw_keyboard: bool,
    ) -> Result<Self, clipcraft_capture::CaptureError> {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            #[cfg(target_os = "linux")]
            if crate::evdev::wanted() {
                return crate::global_shortcuts::start(bindings, allow_raw_keyboard, None);
            }
            let _ = allow_raw_keyboard;
            crate::x11::start_hotkeys(None, bindings)
        }
        #[cfg(windows)]
        {
            let _ = allow_raw_keyboard;
            crate::windows::start_hotkeys(bindings)
        }
        #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
        {
            let _ = bindings;
            let _ = allow_raw_keyboard;
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
        worker: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            backend: Backend::Channel {
                receiver,
                stop,
                worker,
            },
            skipped,
            #[cfg(target_os = "linux")]
            portal_status: None,
            #[cfg(target_os = "linux")]
            overflow: None,
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

    /// Current backend and any portal bindings the desktop did not accept.
    pub fn status(&self) -> HotkeyStatus {
        #[cfg(target_os = "linux")]
        if let Some(status) = &self.portal_status {
            let current = status
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if self.is_stopped() && !matches!(current, HotkeyStatus::Unavailable { .. }) {
                return HotkeyStatus::Unavailable { denied: false };
            }
            return current;
        }
        HotkeyStatus::Ready
    }

    /// Reports a full bounded event queue so the caller can release held push-to-talk keys.
    pub fn take_overflow(&self) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(overflow) = &self.overflow {
            return overflow.swap(false, std::sync::atomic::Ordering::AcqRel);
        }
        false
    }

    /// Ask a listener to stop without blocking the UI thread.
    pub fn request_stop(&self) {
        match &self.backend {
            #[cfg(not(windows))]
            Backend::Channel { stop, .. } => stop.store(true, std::sync::atomic::Ordering::Release),
            #[cfg(windows)]
            Backend::Windows(_) => {}
        }
    }

    /// Whether an asynchronous listener has finished releasing its native resources.
    pub fn is_stopped(&self) -> bool {
        match &self.backend {
            #[cfg(not(windows))]
            Backend::Channel { worker, .. } => worker.is_finished(),
            #[cfg(windows)]
            Backend::Windows(_) => true,
        }
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

    #[test]
    fn modifiers_ignore_case_and_the_plus_key_can_be_bound() {
        let (modifiers, key) = parse_shortcut("ctrl+SHIFT+F8").unwrap();
        assert!(modifiers.ctrl && modifiers.shift && key == "F8");
        let (modifiers, key) = parse_shortcut("Ctrl++").unwrap();
        assert!(modifiers.ctrl && key == "+");
        assert_eq!(parse_shortcut("+").unwrap().1, "+");
        assert!(parse_shortcut("Ctrl++A").is_none());
    }
}
