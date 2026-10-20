//! The portal conversation, against a stand-in for the desktop: a private session bus with a
//! fake `org.freedesktop.portal.ScreenCast` that answers like a real one (a request handle first,
//! the answer afterwards in a `Response` signal). The pictures come from a real PipeWire node
//! made with GStreamer. Skips, saying so, without `dbus-daemon`, a PipeWire server or GStreamer.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::fd::OwnedFd;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clipcraft_capture::CaptureError;
use clipcraft_platform::CaptureOptions;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

struct Reaper(Child);

impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// How the fake chooser ends.
#[derive(Clone, Copy)]
enum Answer {
    Share,
    Cancel,
}

#[derive(Default)]
struct Seen {
    methods: Vec<String>,
    cursor_mode: Option<u32>,
    closed: bool,
}

struct FakeScreenCast {
    connection: Connection,
    answer: Answer,
    node: u32,
    pipewire_socket: std::path::PathBuf,
    seen: Arc<Mutex<Seen>>,
}

impl FakeScreenCast {
    fn respond(&self, request: &ObjectPath<'_>, results: HashMap<String, OwnedValue>, code: u32) {
        let connection = self.connection.clone();
        let path = request.to_owned();
        std::thread::spawn(move || {
            let _ = connection.emit_signal(
                None::<&str>,
                path,
                "org.freedesktop.portal.Request",
                "Response",
                &(code, results),
            );
        });
    }

    fn request_path(&self, token: &str) -> OwnedObjectPath {
        let sender = self
            .connection
            .unique_name()
            .map(|name| name.as_str().trim_start_matches(':').replace('.', "_"))
            .unwrap_or_default();
        // The real portal names the request after the *caller*; this fake only has the token.
        let _ = sender;
        OwnedObjectPath::try_from(format!("/fake/request/{token}")).expect("path")
    }
}

fn token(options: &HashMap<String, OwnedValue>) -> String {
    options
        .get("handle_token")
        .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
        .unwrap_or_default()
}

#[zbus::interface(name = "org.freedesktop.portal.ScreenCast")]
impl FakeScreenCast {
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        4
    }

    fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> OwnedObjectPath {
        self.seen
            .lock()
            .expect("lock")
            .methods
            .push("CreateSession".into());
        let request = self.real_request_path(&header, &token(&options));
        let session = "/fake/session/one".to_owned();
        let mut results = HashMap::new();
        results.insert(
            "session_handle".to_owned(),
            OwnedValue::try_from(Value::from(session)).expect("value"),
        );
        self.respond(&request, results, 0);
        request
    }

    fn select_sources(
        &self,
        _session: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> OwnedObjectPath {
        let mut seen = self.seen.lock().expect("lock");
        seen.methods.push("SelectSources".into());
        seen.cursor_mode = options
            .get("cursor_mode")
            .and_then(|value| u32::try_from(value.try_clone().ok()?).ok());
        drop(seen);
        let request = self.real_request_path(&header, &token(&options));
        self.respond(&request, HashMap::new(), 0);
        request
    }

    fn start(
        &self,
        _session: ObjectPath<'_>,
        _parent: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> OwnedObjectPath {
        self.seen.lock().expect("lock").methods.push("Start".into());
        let request = self.real_request_path(&header, &token(&options));
        match self.answer {
            Answer::Cancel => self.respond(&request, HashMap::new(), 1),
            Answer::Share => {
                let mut properties: HashMap<String, OwnedValue> = HashMap::new();
                properties.insert(
                    "size".to_owned(),
                    OwnedValue::try_from(Value::from((640_i32, 360_i32))).expect("value"),
                );
                let streams = vec![(self.node, properties)];
                let mut results = HashMap::new();
                results.insert(
                    "streams".to_owned(),
                    OwnedValue::try_from(Value::from(streams)).expect("value"),
                );
                self.respond(&request, results, 0);
            }
        }
        request
    }

    fn open_pipe_wire_remote(
        &self,
        _session: ObjectPath<'_>,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        self.seen
            .lock()
            .expect("lock")
            .methods
            .push("OpenPipeWireRemote".into());
        let socket = std::os::unix::net::UnixStream::connect(&self.pipewire_socket)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        Ok(OwnedFd::from(socket).into())
    }
}

impl FakeScreenCast {
    /// The real portal puts the request under the caller's unique name; clients compute the
    /// same path, so the fake must too.
    fn real_request_path(
        &self,
        header: &zbus::message::Header<'_>,
        token: &str,
    ) -> OwnedObjectPath {
        let sender = header
            .sender()
            .map(|name| name.as_str().trim_start_matches(':').replace('.', "_"))
            .unwrap_or_default();
        let _ = self.request_path(token);
        OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .expect("path")
    }
}

struct FakeSession {
    closes: Arc<Mutex<Seen>>,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl FakeSession {
    fn close(&self) {
        self.closes.lock().expect("lock").closed = true;
    }
}

struct Desktop {
    _daemon: Reaper,
    _service: Connection,
    address: String,
    seen: Arc<Mutex<Seen>>,
}

fn desktop(answer: Answer, node: u32, pipewire_socket: std::path::PathBuf) -> Option<Desktop> {
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| eprintln!("desktop step 1: {error:?}"))
        .ok()?;
    let mut address = String::new();
    BufReader::new(daemon.stdout.take()?)
        .read_line(&mut address)
        .map_err(|error| eprintln!("desktop step 2: {error:?}"))
        .ok()?;
    let address = address.trim().to_owned();
    let daemon = Reaper(daemon);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let service = zbus::blocking::connection::Builder::address(address.as_str())
        .map_err(|error| eprintln!("desktop step 3: {error:?}"))
        .ok()?
        .name("org.freedesktop.portal.Desktop")
        .map_err(|error| eprintln!("desktop step 4: {error:?}"))
        .ok()?
        .build()
        .map_err(|error| eprintln!("desktop step 5: {error:?}"))
        .ok()?;
    let fake = FakeScreenCast {
        connection: service.clone(),
        answer,
        node,
        pipewire_socket,
        seen: Arc::clone(&seen),
    };
    service
        .object_server()
        .at("/org/freedesktop/portal/desktop", fake)
        .map_err(|error| eprintln!("desktop step 6: {error:?}"))
        .ok()?;
    service
        .object_server()
        .at(
            "/fake/session/one",
            FakeSession {
                closes: Arc::clone(&seen),
            },
        )
        .map_err(|error| eprintln!("desktop step 7: {error:?}"))
        .ok()?;
    Some(Desktop {
        _daemon: daemon,
        _service: service,
        address,
        seen,
    })
}

fn pipewire_socket() -> Option<std::path::PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let socket = std::path::Path::new(&runtime).join("pipewire-0");
    socket.exists().then_some(socket)
}

/// A GStreamer test picture published as a PipeWire video node; returns its id.
fn producer(name: &str) -> Option<(Reaper, u32)> {
    let child = Command::new("gst-launch-1.0")
        .args([
            "videotestsrc",
            "is-live=true",
            "pattern=solid-color",
            "foreground-color=0xff336699",
            "!",
            "video/x-raw,format=BGRx,width=640,height=360,framerate=30/1",
            "!",
            "pipewiresink",
            "mode=provide",
            &format!("stream-properties=props,media.class=Video/Source,node.name={name}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let reaper = Reaper(child);
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        let listing = Command::new("pw-cli").args(["ls", "Node"]).output().ok()?;
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
        let mut id = None;
        for line in listing.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("id ") {
                id = rest
                    .split(',')
                    .next()
                    .and_then(|number| number.parse().ok());
            } else if line.contains(&format!("\"{name}\"")) {
                return id.map(|id| (reaper, id));
            }
        }
    }
    None
}

#[test]
fn a_person_picks_a_screen_and_its_pictures_arrive() {
    let Some(socket) = pipewire_socket() else {
        eprintln!("skipped: no PipeWire server");
        return;
    };
    let Some((_producer, node)) = producer("cc-portal") else {
        eprintln!("skipped: no GStreamer test source");
        return;
    };
    let Some(desktop) = desktop(Answer::Share, node, socket) else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let mut source = clipcraft_platform::open_wayland_on(
        &desktop.address,
        &CaptureOptions {
            show_cursor: true,
            hdr_to_sdr: true,
        },
    )
    .unwrap_or_else(|error| panic!("the share starts: {error:?}"));
    assert_eq!(source.target().width, 640);
    let end = Instant::now() + Duration::from_secs(5);
    let mut seen_colour = false;
    while Instant::now() < end && !seen_colour {
        let frame = source.capture().expect("a picture");
        let at = (180 * frame.stride) + 320 * 4;
        seen_colour = frame.bgra.get(at..at + 4) == Some(&[0x99, 0x66, 0x33, 255][..]);
    }
    assert!(
        seen_colour,
        "the shared picture arrives with its real colours"
    );
    drop(source);
    let seen = desktop.seen.lock().expect("lock");
    assert!(seen.closed, "stopping the capture ends the share");
    assert_eq!(
        seen.methods,
        [
            "CreateSession",
            "SelectSources",
            "Start",
            "OpenPipeWireRemote"
        ],
        "the conversation happens in the order the portal requires"
    );
    assert_eq!(
        seen.cursor_mode,
        Some(2),
        "the cursor is drawn into the picture"
    );
}

#[test]
fn declining_the_desktops_chooser_is_reported_as_denied() {
    let Some(desktop) = desktop(Answer::Cancel, 0, std::path::PathBuf::from("/nonexistent")) else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let result = clipcraft_platform::open_wayland_on(&desktop.address, &CaptureOptions::default());
    assert!(matches!(result, Err(CaptureError::Denied)));
    assert!(
        desktop
            .seen
            .lock()
            .expect("lock")
            .methods
            .contains(&"Start".to_owned())
    );
}

#[test]
fn a_desktop_without_the_portal_is_no_provider() {
    let Some(desktop) = desktop(Answer::Cancel, 0, std::path::PathBuf::from("/nonexistent")) else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    drop(desktop._service);
    let result = clipcraft_platform::open_wayland_on(&desktop.address, &CaptureOptions::default());
    assert!(result.is_err());
}
