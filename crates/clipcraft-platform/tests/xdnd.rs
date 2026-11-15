//! Drags a file out with the X11 drag source against a stand-in target window on a private
//! virtual display (Xvfb, a test fixture), driven by fake pointer input (the XTest extension).
#![cfg(all(unix, not(target_os = "macos")))]

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::protocol::xtest::fake_input;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

/// Only one drag runs at a time in a process, so these tests take turns.
static ONE_DRAG: Mutex<()> = Mutex::new(());

struct Server {
    child: Child,
    display: String,
}

impl Server {
    fn start(number: u32) -> Option<Self> {
        let display = format!(":{number}");
        let child = Command::new("Xvfb")
            .args([&display, "-screen", "0", "800x600x24", "-nolisten", "tcp"])
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

fn atom(conn: &RustConnection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom
}

/// A window that takes drops the way a chat app does: it accepts the drag, and after the drop it
/// reads the dragged `text/uri-list` and says it is finished. Sends what it read on `done`.
fn target(display: &str, done: mpsc::Sender<String>, ready: mpsc::Sender<()>) {
    let (conn, screen) = RustConnection::connect(Some(display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id().unwrap();
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        window,
        root,
        400,
        100,
        300,
        300,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    )
    .unwrap();
    let aware = atom(&conn, "XdndAware");
    conn.change_property32(PropMode::REPLACE, window, aware, AtomEnum::ATOM, &[5])
        .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();
    let status = atom(&conn, "XdndStatus");
    let position = atom(&conn, "XdndPosition");
    let drop = atom(&conn, "XdndDrop");
    let finished = atom(&conn, "XdndFinished");
    let selection = atom(&conn, "XdndSelection");
    let uri_list = atom(&conn, "text/uri-list");
    let copy = atom(&conn, "XdndActionCopy");
    let property = atom(&conn, "CLIPCRAFT_TEST_DROP");
    let mut source = 0;
    ready.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let Some(event) = conn.poll_for_event().unwrap() else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        match event {
            Event::ClientMessage(message) if message.type_ == position => {
                source = message.data.as_data32()[0];
                let reply = ClientMessageEvent::new(32, source, status, [window, 1, 0, 0, copy]);
                conn.send_event(false, source, EventMask::NO_EVENT, reply)
                    .unwrap();
                conn.flush().unwrap();
            }
            Event::ClientMessage(message) if message.type_ == drop => {
                conn.convert_selection(window, selection, uri_list, property, CURRENT_TIME)
                    .unwrap();
                conn.flush().unwrap();
            }
            Event::SelectionNotify(notify) if notify.property != 0 => {
                let reply = conn
                    .get_property(true, window, property, AtomEnum::ANY, 0, 1024)
                    .unwrap()
                    .reply()
                    .unwrap();
                let text = String::from_utf8_lossy(&reply.value).into_owned();
                let ok = ClientMessageEvent::new(32, source, finished, [window, 1, copy, 0, 0]);
                conn.send_event(false, source, EventMask::NO_EVENT, ok)
                    .unwrap();
                conn.flush().unwrap();
                done.send(text).unwrap();
                return;
            }
            _ => {}
        }
    }
    let _ = done.send("(timed out)".to_owned());
}

fn pointer(conn: &RustConnection, root: u32, kind: u8, detail: u8, x: i16, y: i16) {
    fake_input(conn, kind, detail, 0, root, x, y, 0).unwrap();
    conn.flush().unwrap();
}

#[test]
fn a_dragged_file_reaches_a_window_that_takes_drops_as_a_uri_list() {
    let _turn = ONE_DRAG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(server) = Server::start(74) else {
        eprintln!("Xvfb is not installed; skipping");
        return;
    };
    let folder = std::env::temp_dir().join(format!("clipcraft-xdnd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("A clip #1.mp4");
    std::fs::write(&file, b"clip").unwrap();

    let (done_tx, done_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let display = server.display.clone();
    let thread = std::thread::spawn(move || target(&display, done_tx, ready_tx));
    ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

    let (ctl, screen) = RustConnection::connect(Some(&server.display)).unwrap();
    let root = ctl.setup().roots[screen].root;
    // The button goes down away from the target, as it does on a clip card in the app window.
    pointer(&ctl, root, 6, 0, 50, 50);
    pointer(&ctl, root, 4, 1, 50, 50);
    std::thread::sleep(Duration::from_millis(100));
    clipcraft_platform::start_file_drag_on(&server.display, vec![file.clone()]).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    // Over the target, then let go.
    pointer(&ctl, root, 6, 0, 500, 200);
    std::thread::sleep(Duration::from_millis(400));
    pointer(&ctl, root, 6, 0, 510, 210);
    std::thread::sleep(Duration::from_millis(200));
    pointer(&ctl, root, 5, 1, 510, 210);

    let received = done_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(
        received.trim(),
        format!("file://{}/A%20clip%20%231.mp4", folder.display())
    );
    thread.join().unwrap();
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn letting_go_over_nothing_drops_nothing_and_ends_the_drag() {
    let _turn = ONE_DRAG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(server) = Server::start(75) else {
        eprintln!("Xvfb is not installed; skipping");
        return;
    };
    let folder = std::env::temp_dir().join(format!("clipcraft-xdnd-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("clip.mp4");
    std::fs::write(&file, b"clip").unwrap();
    let (ctl, screen) = RustConnection::connect(Some(&server.display)).unwrap();
    let root = ctl.setup().roots[screen].root;
    pointer(&ctl, root, 6, 0, 50, 50);
    pointer(&ctl, root, 4, 1, 50, 50);
    clipcraft_platform::start_file_drag_on(&server.display, vec![file]).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    pointer(&ctl, root, 5, 1, 50, 50);
    // Nothing to assert beyond not hanging or panicking; give the drag thread time to end.
    std::thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_second_drag_while_one_is_running_is_refused_and_the_next_can_start_after() {
    let _turn = ONE_DRAG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(server) = Server::start(76) else {
        eprintln!("Xvfb is not installed; skipping");
        return;
    };
    let folder = std::env::temp_dir().join(format!("clipcraft-xdnd-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("clip.mp4");
    std::fs::write(&file, b"clip").unwrap();
    let (ctl, screen) = RustConnection::connect(Some(&server.display)).unwrap();
    let root = ctl.setup().roots[screen].root;
    pointer(&ctl, root, 6, 0, 50, 50);
    pointer(&ctl, root, 4, 1, 50, 50);
    clipcraft_platform::start_file_drag_on(&server.display, vec![file.clone()]).unwrap();
    assert!(clipcraft_platform::start_file_drag_on(&server.display, vec![file.clone()]).is_err());
    pointer(&ctl, root, 5, 1, 50, 50);
    // Once the button is up the first drag ends and a new one may start.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pointer(&ctl, root, 4, 1, 50, 50);
        if clipcraft_platform::start_file_drag_on(&server.display, vec![file.clone()]).is_ok() {
            break;
        }
        pointer(&ctl, root, 5, 1, 50, 50);
        assert!(Instant::now() < deadline, "the drag never ended");
        std::thread::sleep(Duration::from_millis(50));
    }
    pointer(&ctl, root, 5, 1, 50, 50);
    std::thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_dir_all(&folder);
}
