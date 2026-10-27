//! Global hotkeys on Linux by reading the keyboards themselves (`/dev/input/event*`).
//!
//! Wayland has no way for one program to grab a key for the whole desktop, and X11 grabs only see
//! keys while an X window (an XWayland game, say) has the focus. Reading the keyboard devices works
//! under every compositor and under X11 too. It needs read access to the devices: membership of the
//! `input` group, or a udev rule that tags keyboards `uaccess` (see `docs/linux.md`). Reading is
//! passive: no key is grabbed or swallowed, and a game still receives it.
//!
//! No `unsafe`: the devices are plain files read with `poll` and `read`, and `/proc/bus/input/devices`
//! says which of them are keyboards.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_capture::CaptureError;
use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::hotkey::{HotkeyBinding, HotkeyEvent, HotkeyListener, Modifiers, parse_shortcut};

/// Most keyboards read at once.
const MAX_DEVICES: usize = 16;
/// How often the device list is looked at again, so a keyboard plugged in later is picked up.
const RESCAN: Duration = Duration::from_secs(3);
/// How long one wait for key events lasts before the stop flag and the rescan timer are checked.
const WAIT_MS: i64 = 200;
/// Bytes of one `struct input_event`: a timestamp, then type, code and value.
const EVENT_BYTES: usize = if cfg!(target_pointer_width = "64") {
    24
} else {
    16
};
/// Where the timestamp ends and the type/code/value begin.
const BODY: usize = EVENT_BYTES - 8;

const EV_KEY: u16 = 1;
const KEY_ENTER: usize = 28;
const KEY_A: usize = 30;
const KEY_SPACE: usize = 57;

/// Whether this session should read keyboards for hotkeys: Wayland sessions, where X11 grabs cannot
/// see keys pressed in other windows.
pub fn wanted() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    set("WAYLAND_DISPLAY")
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland"))
}

/// The Linux key code (`KEY_*`) for a shortcut's key name, using the physical (US) key positions.
pub fn key_code(name: &str) -> Option<u16> {
    let mut chars = name.chars();
    if let (Some(only), None) = (chars.next(), chars.next()) {
        const LETTERS: [u16; 26] = [
            30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47,
            17, 45, 21, 44,
        ];
        if only.is_ascii_uppercase() {
            return LETTERS.get(usize::from(only as u8 - b'A')).copied();
        }
        if only.is_ascii_digit() {
            return Some(match only {
                '0' => 11,
                digit => 2 + (digit as u16 - '1' as u16),
            });
        }
    }
    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()) {
        return match number {
            1..=10 => Some(58 + number),
            11 => Some(87),
            12 => Some(88),
            13..=24 => Some(183 + (number - 13)),
            _ => None,
        };
    }
    Some(match name {
        "Space" => 57,
        "Tab" => 15,
        "Enter" => 28,
        "Backspace" => 14,
        "Delete" => 111,
        "Home" => 102,
        "ArrowLeft" => 105,
        "ArrowUp" => 103,
        "ArrowRight" => 106,
        "ArrowDown" => 108,
        "PageUp" => 104,
        "PageDown" => 109,
        "End" => 107,
        "Insert" => 110,
        "Minus" => 12,
        "Equals" => 13,
        "Comma" => 51,
        "Period" => 52,
        "Slash" => 53,
        "Backtick" => 41,
        _ => return None,
    })
}

/// A shortcut ready to match against key codes.
#[derive(Clone, Copy, Debug)]
struct Registered {
    id: u32,
    modifiers: Modifiers,
    code: u16,
}

/// Follows which modifiers are held and which shortcuts are down, across every keyboard.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    bindings: Vec<Registered>,
    held: HashSet<u16>,
    down: Vec<(u16, u32)>,
}

impl Tracker {
    fn new(bindings: Vec<Registered>) -> Self {
        Self {
            bindings,
            held: HashSet::new(),
            down: Vec::new(),
        }
    }

    fn modifiers(&self) -> Modifiers {
        let any = |codes: [u16; 2]| codes.iter().any(|code| self.held.contains(code));
        Modifiers {
            ctrl: any([29, 97]),
            shift: any([42, 54]),
            alt: any([56, 100]),
        }
    }

    /// Feeds one key event (`value` 1 pressed, 0 released, 2 repeating) and returns the shortcut
    /// events it causes.
    pub(crate) fn feed(&mut self, code: u16, value: i32) -> Vec<HotkeyEvent> {
        let mut out = Vec::new();
        if matches!(code, 29 | 97 | 42 | 54 | 56 | 100) {
            match value {
                0 => {
                    self.held.remove(&code);
                }
                1 => {
                    self.held.insert(code);
                }
                _ => {}
            }
            return out;
        }
        match value {
            1 => {
                let held = self.modifiers();
                for binding in self.bindings.iter().filter(|b| b.code == code) {
                    if binding.modifiers == held {
                        self.down.push((code, binding.id));
                        out.push(HotkeyEvent {
                            id: binding.id,
                            pressed: true,
                        });
                    }
                }
            }
            0 => {
                self.down.retain(|(down, id)| {
                    if *down == code {
                        out.push(HotkeyEvent {
                            id: *id,
                            pressed: false,
                        });
                        false
                    } else {
                        true
                    }
                });
            }
            _ => {}
        }
        out
    }
}

/// The `(type, code, value)` of each whole event in `bytes`.
pub(crate) fn parse_events(bytes: &[u8]) -> Vec<(u16, u16, i32)> {
    bytes
        .chunks_exact(EVENT_BYTES)
        .filter_map(|event| {
            let body = event.get(BODY..)?;
            Some((
                u16::from_ne_bytes([*body.first()?, *body.get(1)?]),
                u16::from_ne_bytes([*body.get(2)?, *body.get(3)?]),
                i32::from_ne_bytes([*body.get(4)?, *body.get(5)?, *body.get(6)?, *body.get(7)?]),
            ))
        })
        .collect()
}

/// Whether bit `bit` is set in a `/proc/bus/input/devices` bitmap line (words most significant
/// first, as many bits per word as this computer's `long`).
fn bit_set(words: &[u64], bit: usize) -> bool {
    let width = usize::BITS as usize;
    let index = bit / width;
    words
        .len()
        .checked_sub(index + 1)
        .and_then(|at| words.get(at))
        .is_some_and(|word| word >> (bit % width) & 1 == 1)
}

/// The `/dev/input/eventN` paths of the keyboards listed in `/proc/bus/input/devices` text: devices
/// handled as `kbd` whose key bitmap has the Enter, A and Space keys, which leaves out mice, power
/// buttons and the like.
pub(crate) fn keyboards_in(listing: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for block in listing.split("\n\n") {
        let mut handlers = "";
        let mut keys: Vec<u64> = Vec::new();
        for line in block.lines() {
            if let Some(rest) = line.strip_prefix("H: Handlers=") {
                handlers = rest;
            } else if let Some(rest) = line.strip_prefix("B: KEY=") {
                keys = rest
                    .split_whitespace()
                    .filter_map(|word| u64::from_str_radix(word, 16).ok())
                    .collect();
            }
        }
        let words: Vec<&str> = handlers.split_whitespace().collect();
        let event = words.iter().find(|word| {
            word.strip_prefix("event")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        });
        if let Some(event) = event
            && words.contains(&"kbd")
            && [KEY_ENTER, KEY_A, KEY_SPACE]
                .iter()
                .all(|key| bit_set(&keys, *key))
        {
            found.push(Path::new("/dev/input").join(event));
        }
    }
    found.truncate(MAX_DEVICES);
    found
}

fn system_keyboards() -> Vec<PathBuf> {
    std::fs::read_to_string("/proc/bus/input/devices")
        .map(|listing| keyboards_in(&listing))
        .unwrap_or_default()
}

fn open(path: &Path) -> Option<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(path)
        .ok()
}

/// Whether any keyboard can be read now, and a sentence saying how to fix it when none can.
pub fn access() -> (bool, String) {
    let keyboards = system_keyboards();
    if keyboards.is_empty() {
        return (
            false,
            "no keyboard was found in /proc/bus/input/devices".to_owned(),
        );
    }
    let readable = keyboards.iter().filter(|path| open(path).is_some()).count();
    if readable > 0 {
        (
            true,
            format!("{readable} of {} keyboard(s) can be read", keyboards.len()),
        )
    } else {
        (
            false,
            "keyboards exist but cannot be read: add your user to the \"input\" group (then sign \
             in again) or install the udev rule described in docs/linux.md"
                .to_owned(),
        )
    }
}

/// Starts reading the system's keyboards for `bindings`.
pub fn start(bindings: Vec<HotkeyBinding>) -> Result<HotkeyListener, CaptureError> {
    start_with(system_keyboards, bindings)
}

pub(crate) fn start_with(
    devices: impl Fn() -> Vec<PathBuf> + Send + 'static,
    bindings: Vec<HotkeyBinding>,
) -> Result<HotkeyListener, CaptureError> {
    let mut registered = Vec::new();
    let mut skipped = Vec::new();
    for binding in &bindings {
        match parse_shortcut(&binding.shortcut).and_then(|(modifiers, key)| {
            Some(Registered {
                id: binding.id,
                modifiers,
                code: key_code(&key)?,
            })
        }) {
            Some(shortcut) => registered.push(shortcut),
            None => skipped.push(binding.shortcut.clone()),
        }
    }
    // Fail now, not silently later, when nothing can be read: the caller falls back to X11 grabs.
    let first: Vec<(PathBuf, File)> = devices()
        .into_iter()
        .filter_map(|path| open(&path).map(|file| (path, file)))
        .collect();
    if first.is_empty() {
        return Err(CaptureError::Denied);
    }
    let (sender, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("evdev-hotkeys".into())
        .spawn(move || listen(devices, first, Tracker::new(registered), sender, &flag))
        .map_err(|_| CaptureError::Failed)?;
    Ok(HotkeyListener::from_channel(receiver, stop, skipped))
}

fn listen(
    devices: impl Fn() -> Vec<PathBuf>,
    first: Vec<(PathBuf, File)>,
    mut tracker: Tracker,
    sender: mpsc::Sender<HotkeyEvent>,
    stop: &AtomicBool,
) {
    let mut open_devices = first;
    let mut last_scan = Instant::now();
    let mut buffer = vec![0_u8; EVENT_BYTES * 64];
    while !stop.load(Ordering::Relaxed) {
        if last_scan.elapsed() >= RESCAN {
            last_scan = Instant::now();
            for path in devices() {
                if open_devices.len() < MAX_DEVICES
                    && !open_devices.iter().any(|(known, _)| *known == path)
                    && let Some(file) = open(&path)
                {
                    open_devices.push((path, file));
                }
            }
        }
        if open_devices.is_empty() {
            std::thread::sleep(Duration::from_millis(WAIT_MS as u64));
            continue;
        }
        let ready: Vec<bool> = {
            let mut fds: Vec<PollFd<'_>> = open_devices
                .iter()
                .map(|(_, file)| PollFd::new(file, PollFlags::IN))
                .collect();
            let wait = Timespec {
                tv_sec: 0,
                tv_nsec: WAIT_MS * 1_000_000,
            };
            if poll(&mut fds, Some(&wait)).is_err() {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            fds.iter()
                .map(|fd| {
                    fd.revents()
                        .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
                })
                .collect()
        };
        let mut gone = Vec::new();
        for (index, ((_, file), ready)) in open_devices.iter().zip(ready).enumerate() {
            if !ready {
                continue;
            }
            match (&*file).read(&mut buffer) {
                Ok(0) => gone.push(index),
                Ok(read) => {
                    for (kind, code, value) in parse_events(buffer.get(..read).unwrap_or(&[])) {
                        if kind != EV_KEY {
                            continue;
                        }
                        for event in tracker.feed(code, value) {
                            if sender.send(event).is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                // The keyboard was unplugged: forget it; the rescan finds it again when it returns.
                Err(_) => gone.push(index),
            }
        }
        for index in gone.into_iter().rev() {
            open_devices.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindings(shortcuts: &[(u32, &str)]) -> Vec<Registered> {
        shortcuts
            .iter()
            .filter_map(|(id, text)| {
                let (modifiers, key) = parse_shortcut(text)?;
                Some(Registered {
                    id: *id,
                    modifiers,
                    code: key_code(&key)?,
                })
            })
            .collect()
    }

    fn event(kind: u16, code: u16, value: i32) -> Vec<u8> {
        let mut bytes = vec![0_u8; BODY];
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&code.to_ne_bytes());
        bytes.extend_from_slice(&value.to_ne_bytes());
        bytes
    }

    #[test]
    fn key_names_map_to_linux_codes() {
        assert_eq!(key_code("F1"), Some(59));
        assert_eq!(key_code("F8"), Some(66));
        assert_eq!(key_code("F10"), Some(68));
        assert_eq!(key_code("F11"), Some(87));
        assert_eq!(key_code("F12"), Some(88));
        assert_eq!(key_code("F13"), Some(183));
        assert_eq!(key_code("F24"), Some(194));
        assert_eq!(key_code("A"), Some(30));
        assert_eq!(key_code("R"), Some(19));
        assert_eq!(key_code("Z"), Some(44));
        assert_eq!(key_code("1"), Some(2));
        assert_eq!(key_code("0"), Some(11));
        assert_eq!(key_code("Space"), Some(57));
        assert_eq!(key_code("Nonsense"), None);
        assert_eq!(key_code("F25"), None);
        assert_eq!(key_code(""), None);
    }

    #[test]
    fn a_plain_key_fires_on_press_and_release_and_ignores_repeats() {
        let mut tracker = Tracker::new(bindings(&[(1, "F8")]));
        assert_eq!(
            tracker.feed(66, 1),
            [HotkeyEvent {
                id: 1,
                pressed: true
            }]
        );
        assert!(tracker.feed(66, 2).is_empty());
        assert_eq!(
            tracker.feed(66, 0),
            [HotkeyEvent {
                id: 1,
                pressed: false
            }]
        );
        assert!(tracker.feed(67, 1).is_empty());
    }

    #[test]
    fn modifiers_must_match_exactly_from_either_side_of_the_keyboard() {
        let mut tracker = Tracker::new(bindings(&[(1, "F8"), (2, "Ctrl+Shift+R")]));
        // Ctrl held: plain F8 must not fire.
        tracker.feed(29, 1);
        assert!(tracker.feed(66, 1).is_empty());
        tracker.feed(66, 0);
        tracker.feed(29, 0);
        // Right Ctrl + left Shift + R fires binding 2 only.
        tracker.feed(97, 1);
        tracker.feed(42, 1);
        assert_eq!(
            tracker.feed(19, 1),
            [HotkeyEvent {
                id: 2,
                pressed: true
            }]
        );
        // Releasing the modifiers first still lets the key's release through.
        tracker.feed(97, 0);
        tracker.feed(42, 0);
        assert_eq!(
            tracker.feed(19, 0),
            [HotkeyEvent {
                id: 2,
                pressed: false
            }]
        );
    }

    #[test]
    fn events_are_read_from_raw_bytes_and_cut_ones_are_ignored() {
        let mut bytes = event(EV_KEY, 66, 1);
        bytes.extend(event(0, 0, 0));
        assert_eq!(parse_events(&bytes), [(EV_KEY, 66, 1), (0, 0, 0)]);
        bytes.truncate(bytes.len() - 3);
        assert_eq!(parse_events(&bytes), [(EV_KEY, 66, 1)]);
        assert!(parse_events(&[1, 2, 3]).is_empty());
        assert!(parse_events(&[]).is_empty());
    }

    #[test]
    fn keyboards_are_told_from_mice_and_buttons_in_the_system_listing() {
        // The key bitmap's words are printed most significant first, one per `long`. The keyboard
        // below has bits 28, 30 and 57 (Enter, A, Space) set: 0x0200000050000000.
        let word = format!("{:x}", (1_u64 << 28) | (1 << 30) | (1 << 57));
        let listing = format!(
            "I: Bus=0011 Vendor=0001 Product=0001 Version=ab41\n\
             N: Name=\"AT Translated Set 2 keyboard\"\n\
             H: Handlers=sysrq kbd event3 leds\n\
             B: PROP=0\n\
             B: EV=120013\n\
             B: KEY={word}\n\
             \n\
             I: Bus=0003 Vendor=046d Product=c077 Version=0111\n\
             N: Name=\"USB Optical Mouse\"\n\
             H: Handlers=mouse0 event5\n\
             B: EV=17\n\
             B: KEY=1f0000 0 0 0 0\n\
             \n\
             I: Bus=0019 Vendor=0000 Product=0001 Version=0000\n\
             N: Name=\"Power Button\"\n\
             H: Handlers=kbd event0\n\
             B: EV=3\n\
             B: KEY=10000000000000 0\n"
        );
        if usize::BITS == 64 {
            assert_eq!(keyboards_in(&listing), [PathBuf::from("/dev/input/event3")]);
        }
        assert!(keyboards_in("").is_empty());
        assert!(keyboards_in("H: Handlers=kbd eventx\nB: KEY=zz").is_empty());
    }

    /// A named pipe stands in for a keyboard device: what is written to it is what the keyboard
    /// would report.
    #[test]
    fn a_listener_reports_shortcuts_from_a_device_file_and_survives_garbage() {
        let directory =
            std::env::temp_dir().join(format!("clipcraft-evdev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("scratch folder");
        let pipe = directory.join("event9");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status();
        if !made.is_ok_and(|status| status.success()) {
            return;
        }
        // Keep a writer open so the reader never sees the end of the file.
        let mut writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe)
            .expect("writer");
        let path = pipe.clone();
        let listener = start_with(
            move || vec![path.clone()],
            vec![
                HotkeyBinding {
                    id: 7,
                    shortcut: "F8".to_owned(),
                },
                HotkeyBinding {
                    id: 8,
                    shortcut: "Nonsense".to_owned(),
                },
            ],
        )
        .expect("a readable device");
        assert_eq!(listener.skipped(), ["Nonsense"]);
        use std::io::Write;
        // Noise first (a sync event, a mouse button, a cut-off event), then the real key.
        let mut bytes = event(0, 0, 0);
        bytes.extend(event(EV_KEY, 272, 1));
        bytes.extend(event(EV_KEY, 66, 1));
        bytes.extend(event(EV_KEY, 66, 0));
        writer.write_all(&bytes).expect("write");
        let started = Instant::now();
        let mut seen = Vec::new();
        while seen.len() < 2 && started.elapsed() < Duration::from_secs(5) {
            if let Some(event) = listener.try_next() {
                seen.push(event);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            seen,
            [
                HotkeyEvent {
                    id: 7,
                    pressed: true
                },
                HotkeyEvent {
                    id: 7,
                    pressed: false
                }
            ]
        );
        // Dropping the listener ends the thread promptly.
        let dropped = Instant::now();
        drop(listener);
        assert!(dropped.elapsed() < Duration::from_secs(1));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn nothing_readable_is_an_error_so_the_caller_can_fall_back() {
        let result = start_with(
            || vec![PathBuf::from("/definitely/not/a/device")],
            vec![HotkeyBinding {
                id: 1,
                shortcut: "F8".to_owned(),
            }],
        );
        assert!(matches!(result, Err(CaptureError::Denied)));
    }
}
