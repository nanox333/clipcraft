//! Dragging a file out of the ClipCraft window into another program, such as a Discord chat.
//!
//! The window library cannot start a drag that leaves the window, so this talks to the system
//! directly: OLE drag and drop on Windows, and the XDND protocol on X11. Wayland and macOS have
//! no way in here yet and report that, so the caller can offer Copy file instead.
//!
//! The pointer is *followed*, not grabbed: the window the drag started in keeps its own mouse
//! grab, so this reads the pointer position and button, and the window sees the button come up
//! as usual when the drag ends.
#![cfg_attr(windows, allow(unsafe_code))]

use std::path::PathBuf;

/// Starts dragging `files` (existing files, already in their final form) from where the pointer
/// is now, until the button is released.
///
/// * Windows: runs the whole drag before returning (it must be on the window's thread), and
///   returns `Ok` when it ended, dropped or not.
/// * Linux on X11: returns at once; the drag runs on its own thread.
/// * Anywhere else: `Err` with the reason, and nothing happens.
pub fn start_file_drag(files: Vec<PathBuf>) -> Result<(), String> {
    check(&files)?;
    let turn = take_turn()?;
    platform::start(files, None, turn)
}

/// As [`start_file_drag`], on the X11 display named `display` (for tests on a virtual display).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn start_file_drag_on(display: &str, files: Vec<PathBuf>) -> Result<(), String> {
    check(&files)?;
    let turn = take_turn()?;
    platform::start(files, Some(display), turn)
}

/// Only one drag at a time: the pointer is followed until the button comes up, so a second drag
/// started on top of the first would only duplicate that work.
static DRAGGING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Holds the right to drag; releasing it (on drop) lets the next drag start.
pub(crate) struct Turn(());

impl Drop for Turn {
    fn drop(&mut self) {
        DRAGGING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

fn take_turn() -> Result<Turn, String> {
    if DRAGGING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        Err("a drag is already in progress".to_owned())
    } else {
        Ok(Turn(()))
    }
}

fn check(files: &[PathBuf]) -> Result<(), String> {
    if files.is_empty() || files.iter().any(|file| !file.is_file()) {
        return Err("there is no file to drag".to_owned());
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;

    use windows::Win32::Foundation::{
        DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
    };
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{
        DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DoDragDrop, IDropSource, IDropSource_Impl,
    };
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{ILFree, SHCreateDataObject, SHParseDisplayName};
    use windows::core::{BOOL, HRESULT, PCWSTR, implement};

    /// Ends the drag when the left button comes up (a drop) or Escape is pressed (a cancel).
    #[implement(IDropSource)]
    struct Source;

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(
            &self,
            escape_pressed: BOOL,
            key_state: MODIFIERKEYS_FLAGS,
        ) -> HRESULT {
            if escape_pressed.as_bool() {
                DRAGDROP_S_CANCEL
            } else if key_state.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    pub fn start(
        files: Vec<PathBuf>,
        _display: Option<&str>,
        _turn: super::Turn,
    ) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        // One list entry per file, each parsed by the shell into the identifier it uses.
        let mut lists: Vec<*mut ITEMIDLIST> = Vec::new();
        let result = (|| {
            for file in &files {
                let absolute = std::path::absolute(file).map_err(|error| error.to_string())?;
                let mut wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
                if wide.contains(&0) {
                    return Err("the file name is not valid".to_owned());
                }
                wide.push(0);
                let mut list: *mut ITEMIDLIST = std::ptr::null_mut();
                // SAFETY: `wide` is a zero-terminated path that outlives the call, and `list` is
                // a valid place for the shell to put the identifier it allocates.
                unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut list, 0, None) }
                    .map_err(|error| format!("Windows could not read the file: {error}"))?;
                lists.push(list);
            }
            let items: Vec<*const ITEMIDLIST> =
                lists.iter().map(|list| list.cast_const()).collect();
            // SAFETY: `items` holds as many valid absolute identifiers as it says, kept alive
            // until after the drag; no parent folder is given, so they are read as absolute.
            let data: IDataObject =
                unsafe { SHCreateDataObject(None, Some(&items), None::<&IDataObject>) }
                    .map_err(|error| format!("Windows could not prepare the drag: {error}"))?;
            let source: IDropSource = Source.into();
            let mut effect = DROPEFFECT(0);
            // SAFETY: both objects are live COM objects for the length of the call, and `effect`
            // is a valid place for the result. The call runs its own message loop until the drop.
            let outcome = unsafe {
                DoDragDrop(
                    &data,
                    &source,
                    DROPEFFECT_COPY | DROPEFFECT_LINK,
                    &mut effect,
                )
            };
            // A drop, a cancel and a refusing target are all ways a drag ends.
            if outcome == DRAGDROP_S_DROP || outcome == DRAGDROP_S_CANCEL || outcome.is_ok() {
                Ok(())
            } else {
                Err(format!("the drag failed ({:#x})", outcome.0))
            }
        })();
        for list in lists {
            // SAFETY: each pointer came from `SHParseDisplayName` and is freed once.
            unsafe { ILFree(Some(list.cast_const())) };
        }
        result
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::path::PathBuf;

    pub fn start(
        files: Vec<PathBuf>,
        display: Option<&str>,
        turn: super::Turn,
    ) -> Result<(), String> {
        if display.is_none() && std::env::var_os("DISPLAY").is_none_or(|value| value.is_empty()) {
            return Err("dragging out of the window needs an X11 session".to_owned());
        }
        // Start the connection here so a missing server is reported to the caller.
        let drag = super::xdnd::Drag::new(&files, display)?;
        std::thread::Builder::new()
            .name("clipcraft-drag".to_owned())
            .spawn(move || {
                let _turn = turn;
                let _ = drag.run();
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    pub fn start(
        _files: Vec<PathBuf>,
        _display: Option<&str>,
        _turn: super::Turn,
    ) -> Result<(), String> {
        Err("dragging out of the window is not available on macOS yet".to_owned())
    }
}

/// The source side of the X11 drag-and-drop protocol (XDND version 5).
#[cfg(all(unix, not(target_os = "macos")))]
mod xdnd {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask,
        PropMode, SelectionNotifyEvent, Window, WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;
    use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

    /// Newest protocol version this source speaks.
    const VERSION: u32 = 5;
    /// How long to wait for the target to finish reading after the drop.
    const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
    /// The longest a drag may last: a pointer that never reports the button coming up must not
    /// keep the drag (and its polling) alive for ever.
    const LONGEST_DRAG: Duration = Duration::from_secs(300);
    /// How often the pointer is looked at.
    const POLL: Duration = Duration::from_millis(8);
    /// The most windows to look through, from the root down, for one that takes drops.
    const MAX_DEPTH: usize = 12;

    struct Atoms {
        aware: Atom,
        enter: Atom,
        position: Atom,
        status: Atom,
        leave: Atom,
        drop: Atom,
        finished: Atom,
        selection: Atom,
        proxy: Atom,
        copy: Atom,
        uri_list: Atom,
        targets: Atom,
    }

    pub struct Drag {
        conn: RustConnection,
        root: Window,
        helper: Window,
        atoms: Atoms,
        uris: Vec<u8>,
    }

    fn intern(conn: &RustConnection, name: &str) -> Result<Atom, String> {
        conn.intern_atom(false, name.as_bytes())
            .map_err(|error| error.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|error| error.to_string())
    }

    /// `file:///home/me/a%20clip.mp4` for an absolute path.
    fn file_uri(path: &std::path::Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let mut uri = String::from("file://");
        for byte in path.as_os_str().as_bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                    uri.push(char::from(*byte));
                }
                other => uri.push_str(&format!("%{other:02X}")),
            }
        }
        uri
    }

    impl Drag {
        pub fn new(files: &[PathBuf], display: Option<&str>) -> Result<Self, String> {
            let (conn, screen) =
                RustConnection::connect(display).map_err(|error| error.to_string())?;
            let root = conn.setup().roots.get(screen).ok_or("no screen")?.root;
            let atoms = Atoms {
                aware: intern(&conn, "XdndAware")?,
                enter: intern(&conn, "XdndEnter")?,
                position: intern(&conn, "XdndPosition")?,
                status: intern(&conn, "XdndStatus")?,
                leave: intern(&conn, "XdndLeave")?,
                drop: intern(&conn, "XdndDrop")?,
                finished: intern(&conn, "XdndFinished")?,
                selection: intern(&conn, "XdndSelection")?,
                proxy: intern(&conn, "XdndProxy")?,
                copy: intern(&conn, "XdndActionCopy")?,
                uri_list: intern(&conn, "text/uri-list")?,
                targets: intern(&conn, "TARGETS")?,
            };
            // A small hidden window that owns the selection and receives the target's replies.
            let helper = conn.generate_id().map_err(|error| error.to_string())?;
            conn.create_window(
                COPY_DEPTH_FROM_PARENT,
                helper,
                root,
                -10,
                -10,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )
            .map_err(|error| error.to_string())?;
            let mut uris = Vec::new();
            for file in files {
                let absolute = std::path::absolute(file).map_err(|error| error.to_string())?;
                uris.extend_from_slice(file_uri(&absolute).as_bytes());
                uris.extend_from_slice(b"\r\n");
            }
            conn.set_selection_owner(helper, atoms.selection, CURRENT_TIME)
                .map_err(|error| error.to_string())?;
            conn.flush().map_err(|error| error.to_string())?;
            Ok(Self {
                conn,
                root,
                helper,
                atoms,
                uris,
            })
        }

        /// The deepest window under the pointer that takes drops, with the window that should
        /// receive the messages (its proxy, if it has one) and the protocol version it speaks.
        fn target_at(&self, x: i16, y: i16) -> Option<(Window, Window, u32)> {
            let mut window = self.root;
            let mut found = None;
            for _ in 0..MAX_DEPTH {
                let reply = self.conn.query_pointer(window).ok()?.reply().ok()?;
                let child = reply.child;
                if child == x11rb::NONE {
                    break;
                }
                window = child;
                if let Some(version) = self.aware_version(window) {
                    let receiver = self.proxy_of(window).unwrap_or(window);
                    found = Some((window, receiver, version));
                }
            }
            let _ = (x, y);
            found
        }

        fn aware_version(&self, window: Window) -> Option<u32> {
            let reply = self
                .conn
                .get_property(false, window, self.atoms.aware, AtomEnum::ATOM, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            reply.value32()?.next()
        }

        fn proxy_of(&self, window: Window) -> Option<Window> {
            let reply = self
                .conn
                .get_property(false, window, self.atoms.proxy, AtomEnum::WINDOW, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            reply.value32()?.next().filter(|proxy| *proxy != 0)
        }

        fn message(&self, to: Window, kind: Atom, data: [u32; 5]) {
            let event = ClientMessageEvent::new(32, to, kind, data);
            let _ = self.conn.send_event(false, to, EventMask::NO_EVENT, event);
            let _ = self.conn.flush();
        }

        /// Answers a request for the dragged data (the target reads it after the drop).
        fn serve(&self, request: x11rb::protocol::xproto::SelectionRequestEvent) {
            let mut property = request.property;
            if property == x11rb::NONE {
                property = request.target;
            }
            let answered = if request.target == self.atoms.targets {
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        AtomEnum::ATOM,
                        &[self.atoms.targets, self.atoms.uri_list],
                    )
                    .is_ok()
            } else if request.target == self.atoms.uri_list {
                self.conn
                    .change_property8(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        self.atoms.uri_list,
                        &self.uris,
                    )
                    .is_ok()
            } else {
                false
            };
            let notify = SelectionNotifyEvent {
                response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: request.time,
                requestor: request.requestor,
                selection: request.selection,
                target: request.target,
                property: if answered { property } else { x11rb::NONE },
            };
            let _ = self
                .conn
                .send_event(false, request.requestor, EventMask::NO_EVENT, notify);
            let _ = self.conn.flush();
        }

        /// Follows the pointer until the left button comes up, telling the window under it about
        /// the drag, then drops on it.
        pub fn run(self) -> Result<(), String> {
            let mut target: Option<(Window, Window, u32)> = None;
            let mut accepted = false;
            let mut last_position = Instant::now() - POLL;
            let mut waiting_since: Option<Instant> = None;
            let mut dropped = false;
            let started = Instant::now();
            loop {
                if started.elapsed() > LONGEST_DRAG {
                    return Ok(());
                }
                while let Some(event) = self
                    .conn
                    .poll_for_event()
                    .map_err(|error| error.to_string())?
                {
                    match event {
                        Event::SelectionRequest(request) => self.serve(request),
                        Event::ClientMessage(message) if message.type_ == self.atoms.status => {
                            let data = message.data.as_data32();
                            if target.is_some_and(|(window, _, _)| window == data[0]) {
                                accepted = data[1] & 1 == 1;
                            }
                        }
                        Event::ClientMessage(message) if message.type_ == self.atoms.finished => {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                if dropped {
                    // After the drop the target reads the data; wait for it to say it is done.
                    if waiting_since.is_some_and(|since| since.elapsed() > FINISH_TIMEOUT) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                let pointer = self
                    .conn
                    .query_pointer(self.root)
                    .map_err(|error| error.to_string())?
                    .reply()
                    .map_err(|error| error.to_string())?;
                let (x, y) = (pointer.root_x, pointer.root_y);
                let button_down = u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) != 0;
                let now = self.target_at(x, y);
                if now.map(|(window, _, _)| window) != target.map(|(window, _, _)| window) {
                    if let Some((_, receiver, _)) = target {
                        self.message(receiver, self.atoms.leave, [self.helper, 0, 0, 0, 0]);
                    }
                    accepted = false;
                    target = now;
                    if let Some((_, receiver, version)) = target {
                        let used = version.min(VERSION);
                        self.message(
                            receiver,
                            self.atoms.enter,
                            [self.helper, used << 24, self.atoms.uri_list, 0, 0],
                        );
                    }
                    last_position = Instant::now() - POLL;
                }
                if let Some((_, receiver, _)) = target
                    && last_position.elapsed() >= Duration::from_millis(30)
                {
                    last_position = Instant::now();
                    let packed = ((x as u16 as u32) << 16) | (y as u16 as u32);
                    self.message(
                        receiver,
                        self.atoms.position,
                        [self.helper, 0, packed, CURRENT_TIME, self.atoms.copy],
                    );
                }
                if !button_down {
                    match target {
                        Some((_, receiver, _)) if accepted => {
                            self.message(
                                receiver,
                                self.atoms.drop,
                                [self.helper, 0, CURRENT_TIME, 0, 0],
                            );
                            dropped = true;
                            waiting_since = Some(Instant::now());
                        }
                        Some((_, receiver, _)) => {
                            self.message(receiver, self.atoms.leave, [self.helper, 0, 0, 0, 0]);
                            return Ok(());
                        }
                        None => return Ok(()),
                    }
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    #[test]
    fn a_missing_file_or_an_empty_list_is_refused_before_anything_starts() {
        assert!(super::start_file_drag(Vec::new()).is_err());
        assert!(super::start_file_drag(vec!["/definitely/not/here.mp4".into()]).is_err());
    }
}
