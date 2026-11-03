//! Runs the X11 provider against a private virtual display (Xvfb, used only as a test fixture).
#![cfg(all(unix, not(target_os = "macos")))]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use clipcraft_capture::{CaptureError, TargetKind};
use clipcraft_platform::window_for_process_on;
use clipcraft_platform::{HotkeyBinding, HotkeyListener, list_targets_on, open_on};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, WindowClass};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

struct Server {
    child: Child,
    display: String,
}

impl Server {
    /// Starts Xvfb on a free display number, or `None` when it is not installed.
    fn start(number: u32) -> Option<Self> {
        let display = format!(":{number}");
        let child = Command::new("Xvfb")
            .args([
                &display,
                "-screen",
                "0",
                "640x480x24",
                "-nolisten",
                "tcp",
                "-noreset",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let server = Self { child, display };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if RustConnection::connect(Some(&server.display)).is_ok() {
                return Some(server);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Covers the whole screen with one colour (0xRRGGBB) using a full-screen window.
fn paint_root(display: &str, colour: u32) {
    make_window_at(display, 0, 0, 640, 480, colour);
}

/// Makes and maps a window of one colour, returning its id.
fn make_window(display: &str, width: u16, height: u16, colour: u32) -> u32 {
    make_window_at(display, 10, 10, width, height, colour)
}

fn make_window_at(display: &str, x: i16, y: i16, width: u16, height: u16, colour: u32) -> u32 {
    let (conn, screen) = RustConnection::connect(Some(display)).unwrap();
    let screen = &conn.setup().roots[screen];
    let window = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        window,
        screen.root,
        x,
        y,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().background_pixel(colour),
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();
    conn.get_input_focus().unwrap().reply().unwrap();
    // Keep the connection (and so the window) alive for the test by leaking it deliberately.
    std::mem::forget(conn);
    window
}

fn pixel(frame: &clipcraft_capture::CaptureFrame, x: u32, y: u32) -> [u8; 3] {
    let at = y as usize * frame.stride + x as usize * 4;
    [frame.bgra[at + 2], frame.bgra[at + 1], frame.bgra[at]]
}

#[test]
fn a_monitor_is_listed_and_captured_with_the_right_colours() {
    let Some(server) = Server::start(61) else {
        return;
    };
    paint_root(&server.display, 0xff8000);
    let targets = list_targets_on(&server.display).unwrap();
    let monitor = targets
        .iter()
        .find(|t| t.kind == TargetKind::Display)
        .expect("a monitor");
    assert_eq!((monitor.width, monitor.height), (640, 480));
    let mut source = open_on(&server.display, &monitor.id).unwrap();
    let frame = source.capture().unwrap();
    frame.validate().unwrap();
    assert_eq!((frame.width, frame.height), (640, 480));
    assert_eq!(pixel(&frame, 5, 5), [0xff, 0x80, 0x00]);
    assert_eq!(pixel(&frame, 639, 479), [0xff, 0x80, 0x00]);
    // The picture follows the screen.
    paint_root(&server.display, 0x0000ff);
    assert_eq!(
        pixel(&source.capture().unwrap(), 100, 100),
        [0x00, 0x00, 0xff]
    );
}

/// The picture taken through shared memory is byte for byte what the plain protocol returns.
#[test]
fn shared_memory_pictures_equal_the_plain_protocol_pictures() {
    let Some(server) = Server::start(67) else {
        return;
    };
    paint_root(&server.display, 0x336699);
    let _ = make_window_at(&server.display, 40, 30, 200, 120, 0xcc3300);
    let monitor = list_targets_on(&server.display)
        .unwrap()
        .into_iter()
        .find(|t| t.kind == TargetKind::Display)
        .expect("a monitor");
    let mut source = open_on(&server.display, &monitor.id).unwrap();
    // Twice: the second picture reuses the shared segment.
    for round in 0..2 {
        let frame = source.capture().unwrap();
        let (conn, screen) = RustConnection::connect(Some(&server.display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let plain = conn
            .get_image(
                x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
                root,
                0,
                0,
                frame.width as u16,
                frame.height as u16,
                !0,
            )
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(plain.data.len(), frame.bgra.len());
        for (index, (a, b)) in frame
            .bgra
            .chunks_exact(4)
            .zip(plain.data.chunks_exact(4))
            .enumerate()
        {
            assert_eq!(
                a[..3],
                b[..3],
                "round {round}: pixel {index} (x {}, y {}) differs",
                index % frame.width as usize,
                index / frame.width as usize
            );
            assert_eq!(a[3], 255);
        }
    }
}

/// A window that changes size between pictures gets a new shared segment, not a torn picture.
#[test]
fn a_window_that_changes_size_is_still_captured_whole() {
    let Some(server) = Server::start(68) else {
        return;
    };
    paint_root(&server.display, 0x101010);
    let window = make_window(&server.display, 100, 80, 0x00ff00);
    let mut source = open_on(&server.display, &format!("window:{window:x}")).unwrap();
    let first = source.capture().unwrap();
    assert_eq!((first.width, first.height), (100, 80));
    let (conn, _) = RustConnection::connect(Some(&server.display)).unwrap();
    conn.configure_window(
        window,
        &x11rb::protocol::xproto::ConfigureWindowAux::new()
            .width(160)
            .height(120),
    )
    .unwrap();
    conn.flush().unwrap();
    conn.get_input_focus().unwrap().reply().unwrap();
    let second = source.capture().unwrap();
    assert_eq!((second.width, second.height), (160, 120));
    assert_eq!(second.bgra.len(), 160 * 120 * 4);
    assert_eq!(pixel(&second, 80, 60), [0x00, 0xff, 0x00]);
}

#[test]
fn a_window_is_captured_alone_and_its_loss_is_reported() {
    let Some(server) = Server::start(62) else {
        return;
    };
    paint_root(&server.display, 0x202020);
    let window = make_window(&server.display, 120, 90, 0x00ff00);
    let mut source = open_on(&server.display, &format!("window:{window:x}")).unwrap();
    let frame = source.capture().unwrap();
    assert_eq!((frame.width, frame.height), (120, 90));
    assert_eq!(pixel(&frame, 60, 45), [0x00, 0xff, 0x00]);
    assert_eq!(source.target().kind, TargetKind::Window);

    // Destroy it from another connection.
    let (conn, _) = RustConnection::connect(Some(&server.display)).unwrap();
    conn.destroy_window(window).unwrap();
    conn.flush().unwrap();
    conn.get_input_focus().unwrap().reply().unwrap();
    assert_eq!(source.capture().unwrap_err(), CaptureError::TargetGone);
}

#[test]
fn bad_targets_and_missing_displays_are_errors_not_panics() {
    let Some(server) = Server::start(63) else {
        return;
    };
    for id in [
        "",
        "window:zz",
        "monitor:1,2,3",
        "monitor:0,0,99999999,5",
        "nonsense",
    ] {
        assert!(open_on(&server.display, id).is_err(), "{id:?}");
    }
    assert_eq!(
        list_targets_on(":59").unwrap_err(),
        CaptureError::NoProvider
    );
    assert!(open_on(":59", "monitor:0,0,10,10").is_err());
}

#[test]
fn a_new_listener_can_register_after_the_previous_listener_stops() {
    let Some(server) = Server::start(69) else {
        return;
    };
    let bindings = vec![
        HotkeyBinding {
            id: 1,
            shortcut: "F8".into(),
        },
        HotkeyBinding {
            id: 2,
            shortcut: "F9".into(),
        },
    ];
    let first = HotkeyListener::start_on(&server.display, bindings.clone()).unwrap();
    assert!(first.skipped().is_empty());
    first.request_stop();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !first.is_stopped() {
        assert!(
            Instant::now() < deadline,
            "listener did not release its grabs"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(first);
    let next = HotkeyListener::start_on(&server.display, bindings).unwrap();
    assert!(
        next.skipped().is_empty(),
        "the old listener still owns a shortcut"
    );
}

#[test]
fn a_global_hotkey_fires_on_press_and_release_even_with_lock_keys_on() {
    let Some(server) = Server::start(64) else {
        return;
    };
    if Command::new("xdotool").arg("version").output().is_err() {
        return;
    }
    let listener = HotkeyListener::start_on(
        &server.display,
        vec![
            HotkeyBinding {
                id: 1,
                shortcut: "F8".to_owned(),
            },
            HotkeyBinding {
                id: 2,
                shortcut: "Ctrl+Shift+R".to_owned(),
            },
            HotkeyBinding {
                id: 3,
                shortcut: "Nonsense".to_owned(),
            },
        ],
    )
    .unwrap();
    assert_eq!(listener.skipped(), ["Nonsense"]);
    let press = |keys: &str| {
        Command::new("xdotool")
            .env("DISPLAY", &server.display)
            .args(["key", keys])
            .status()
            .unwrap();
    };
    let wait = |id: u32, pressed: bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(event) = listener.try_next()
                && event.id == id
                && event.pressed == pressed
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    };
    press("F8");
    assert!(wait(1, true) && wait(1, false), "F8 not seen");
    press("ctrl+shift+r");
    assert!(wait(2, true), "Ctrl+Shift+R not seen");
}

#[test]
fn the_focused_window_is_found_through_the_window_managers_hint() {
    let Some(server) = Server::start(65) else {
        return;
    };
    // No window manager runs here, so there is no hint yet.
    assert_eq!(
        clipcraft_platform::active_window_on(&server.display).unwrap(),
        None
    );
    let window = make_window(&server.display, 200, 100, 0x336699);
    // Publish the hint the way a window manager does.
    let (conn, screen) = RustConnection::connect(Some(&server.display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let active = atom(b"_NET_ACTIVE_WINDOW");
    let net_name = atom(b"_NET_WM_NAME");
    let utf8 = atom(b"UTF8_STRING");
    conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        root,
        active,
        x11rb::protocol::xproto::AtomEnum::WINDOW,
        &[window],
    )
    .unwrap();
    conn.change_property8(
        x11rb::protocol::xproto::PropMode::REPLACE,
        window,
        net_name,
        utf8,
        "My Game".as_bytes(),
    )
    .unwrap();
    conn.flush().unwrap();
    conn.get_input_focus().unwrap().reply().unwrap();
    let found = clipcraft_platform::active_window_on(&server.display)
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "My Game");
    assert_eq!((found.width, found.height), (200, 100));
    assert_eq!(found.id, format!("window:{window:x}"));
    // The hint can be opened and captured.
    let mut source = open_on(&server.display, &found.id).unwrap();
    assert_eq!(source.capture().unwrap().width, 200);
}

#[test]
fn a_game_window_is_selected_by_its_exact_process_id() {
    let Some(server) = Server::start(66) else {
        return;
    };
    let window = make_window(&server.display, 200, 100, 0x336699);
    let process_id = 42_424;
    let (conn, screen) = RustConnection::connect(Some(&server.display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let clients = atom(b"_NET_CLIENT_LIST");
    let pid = atom(b"_NET_WM_PID");
    conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        root,
        clients,
        x11rb::protocol::xproto::AtomEnum::WINDOW,
        &[window],
    )
    .unwrap();
    conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        window,
        pid,
        x11rb::protocol::xproto::AtomEnum::CARDINAL,
        &[process_id],
    )
    .unwrap();
    conn.flush().unwrap();
    conn.get_input_focus().unwrap().reply().unwrap();

    let found = window_for_process_on(&server.display, process_id)
        .unwrap()
        .unwrap();
    assert_eq!(found.id, format!("window:{window:x}"));
    assert_eq!((found.width, found.height), (200, 100));
    assert!(
        window_for_process_on(&server.display, process_id + 1)
            .unwrap()
            .is_none()
    );
}
