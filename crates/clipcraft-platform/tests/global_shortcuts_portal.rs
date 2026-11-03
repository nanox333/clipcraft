//! Private D-Bus integration test; no physical keyboard or desktop permissions required.
#![cfg(target_os = "linux")]

use clipcraft_platform::{HotkeyBinding, HotkeyEvent, HotkeyStatus, start_wayland_hotkeys_on};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const PATH: &str = "/org/freedesktop/portal/desktop";
const GLOBAL: &str = "org.freedesktop.portal.GlobalShortcuts";
const SESSION: &str = "/fake/session/one";

struct Reaper(Child);
impl Drop for Reaper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Default)]
struct Seen {
    methods: Vec<String>,
    close_count: usize,
    triggers: Vec<String>,
    request_ready: bool,
    request_closed: usize,
}

#[derive(Clone, Copy)]
enum Answer {
    Accept,
    Deny,
    Wait,
    Empty,
}

struct FakePortal {
    connection: Connection,
    answer: Answer,
    seen: Arc<Mutex<Seen>>,
}
impl FakePortal {
    fn path(
        header: &zbus::message::Header<'_>,
        options: &HashMap<String, OwnedValue>,
    ) -> OwnedObjectPath {
        let sender = header
            .sender()
            .unwrap()
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let token = options
            .get("handle_token")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap();
        OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .unwrap()
    }
    fn respond(&self, path: &OwnedObjectPath, code: u32, results: HashMap<String, OwnedValue>) {
        let connection = self.connection.clone();
        let path = path.clone();
        std::thread::spawn(move || {
            connection
                .emit_signal(
                    None::<&str>,
                    path,
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(code, results),
                )
                .unwrap();
        });
    }
}

#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl FakePortal {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
    fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> OwnedObjectPath {
        self.seen.lock().unwrap().methods.push("create".into());
        let path = Self::path(&header, &options);
        let results = HashMap::from([(
            "session_handle".to_owned(),
            OwnedValue::try_from(Value::from(SESSION.to_owned())).unwrap(),
        )]);
        self.respond(&path, 0, results);
        path
    }
    fn bind_shortcuts(
        &self,
        _session: ObjectPath<'_>,
        shortcuts: Vec<(String, HashMap<String, OwnedValue>)>,
        _parent: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> OwnedObjectPath {
        let path = Self::path(&header, &options);
        let mut seen = self.seen.lock().unwrap();
        seen.methods.push("bind".into());
        seen.triggers = shortcuts
            .iter()
            .map(|(_, p)| {
                p.get("preferred_trigger")
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                    .unwrap()
            })
            .collect();
        drop(seen);
        match self.answer {
            Answer::Deny => self.respond(&path, 1, HashMap::new()),
            Answer::Wait => {
                let connection = self.connection.clone();
                let path = path.clone();
                let seen = self.seen.clone();
                std::thread::spawn(move || {
                    connection
                        .object_server()
                        .at(path, FakeRequest(seen.clone()))
                        .unwrap();
                    seen.lock().unwrap().request_ready = true;
                });
            }
            Answer::Empty => {
                let bound: Vec<(String, HashMap<&str, Value<'_>>)> = Vec::new();
                let results = HashMap::from([(
                    "shortcuts".to_owned(),
                    OwnedValue::try_from(Value::from(bound)).unwrap(),
                )]);
                self.respond(&path, 0, results);
            }
            Answer::Accept => {
                let bound: Vec<(String, HashMap<&str, Value<'_>>)> = shortcuts
                    .into_iter()
                    .take(2)
                    .map(|(id, _)| {
                        (
                            id,
                            HashMap::from([(
                                "trigger_description",
                                Value::from("Desktop shortcut"),
                            )]),
                        )
                    })
                    .collect();
                let results = HashMap::from([(
                    "shortcuts".to_owned(),
                    OwnedValue::try_from(Value::from(bound)).unwrap(),
                )]);
                self.respond(&path, 0, results);
            }
        }
        path
    }
}

struct FakeRequest(Arc<Mutex<Seen>>);
#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl FakeRequest {
    fn close(&self) {
        self.0.lock().unwrap().request_closed += 1;
    }
}

struct FakeSession(Arc<Mutex<Seen>>);
#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl FakeSession {
    fn close(&self) {
        self.0.lock().unwrap().close_count += 1;
    }
}

struct Desktop {
    _daemon: Reaper,
    service: Connection,
    address: String,
    seen: Arc<Mutex<Seen>>,
}
fn desktop(answer: Answer) -> Desktop {
    let mut daemon = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon");
    let mut address = String::new();
    BufReader::new(daemon.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim().to_owned();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let service = zbus::blocking::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .build()
        .unwrap();
    service
        .object_server()
        .at(
            PATH,
            FakePortal {
                connection: service.clone(),
                answer,
                seen: seen.clone(),
            },
        )
        .unwrap();
    service
        .object_server()
        .at(SESSION, FakeSession(seen.clone()))
        .unwrap();
    Desktop {
        _daemon: Reaper(daemon),
        service,
        address,
        seen,
    }
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn bindings() -> Vec<HotkeyBinding> {
    [(1, "F8"), (16, "Ctrl+Shift+A"), (21, "F11")]
        .into_iter()
        .map(|(id, shortcut)| HotkeyBinding {
            id,
            shortcut: shortcut.into(),
        })
        .collect()
}
fn emit(desktop: &Desktop, session: &str, signal: &str, id: &str) {
    let session = ObjectPath::try_from(session).unwrap();
    desktop
        .service
        .emit_signal(
            None::<&str>,
            PATH,
            GLOBAL,
            signal,
            &(session, id, 1_u64, HashMap::<String, OwnedValue>::new()),
        )
        .unwrap();
}

#[test]
fn binds_partial_shortcuts_forwards_press_and_release_and_closes() {
    let desktop = desktop(Answer::Accept);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(listener.status(), HotkeyStatus::Portal { .. }));
    let HotkeyStatus::Portal { unbound, assigned } = listener.status() else {
        panic!("portal not active")
    };
    assert_eq!(unbound, ["F11"]);
    assert_eq!(assigned.len(), 2);
    assert!(
        assigned
            .iter()
            .all(|(_, actual)| actual == "Desktop shortcut")
    );
    assert_eq!(
        desktop.seen.lock().unwrap().triggers,
        ["F8", "CTRL+SHIFT+a", "F11"]
    );
    emit(&desktop, "/fake/session/other", "Activated", "shortcut_1");
    emit(&desktop, SESSION, "Activated", "shortcut_21");
    emit(&desktop, SESSION, "Activated", "shortcut_16");
    emit(&desktop, SESSION, "Deactivated", "shortcut_16");
    let mut events = Vec::new();
    wait_for(|| {
        while let Some(event) = listener.try_next() {
            events.push(event);
        }
        events.len() == 2
    });
    assert_eq!(
        events,
        [
            HotkeyEvent {
                id: 16,
                pressed: true
            },
            HotkeyEvent {
                id: 16,
                pressed: false
            }
        ]
    );
    drop(listener);
    wait_for(|| desktop.seen.lock().unwrap().close_count == 1);
}

#[test]
fn denial_and_cancelled_pending_chooser_release_session() {
    let denied = desktop(Answer::Deny);
    let listener = start_wayland_hotkeys_on(&denied.address, bindings()).unwrap();
    wait_for(|| listener.status() == HotkeyStatus::Unavailable { denied: true });
    assert!(!listener.take_overflow(), "no key was active before denial");
    wait_for(|| denied.seen.lock().unwrap().close_count == 1);
    drop(listener);

    let waiting = desktop(Answer::Wait);
    let listener = start_wayland_hotkeys_on(&waiting.address, bindings()).unwrap();
    wait_for(|| waiting.seen.lock().unwrap().request_ready);
    assert_eq!(listener.status(), HotkeyStatus::Pending);
    listener.request_stop();
    wait_for(|| listener.is_stopped());
    drop(listener);
    wait_for(|| waiting.seen.lock().unwrap().close_count == 1);
    assert_eq!(waiting.seen.lock().unwrap().request_closed, 1);
}

#[test]
fn portal_disappearance_and_queue_overflow_fail_closed() {
    let desktop = desktop(Answer::Accept);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(listener.status(), HotkeyStatus::Portal { .. }));
    // Keep the UI from consuming events, then exercise the bounded queue.
    for _ in 0..100 {
        emit(&desktop, SESSION, "Activated", "shortcut_16");
        emit(&desktop, SESSION, "Deactivated", "shortcut_16");
    }
    wait_for(|| listener.take_overflow());
    while listener.try_next().is_some() {}
    emit(&desktop, SESSION, "Activated", "shortcut_16");
    wait_for(|| listener.try_next().is_some());
    desktop
        .service
        .release_name("org.freedesktop.portal.Desktop")
        .unwrap();
    wait_for(|| listener.status() == HotkeyStatus::Unavailable { denied: false });
    assert!(
        listener.take_overflow(),
        "disconnect must release held push-to-talk"
    );
}

#[test]
fn malformed_bindings_are_refused_before_opening_a_session() {
    let desktop = desktop(Answer::Accept);
    assert!(
        start_wayland_hotkeys_on(
            &desktop.address,
            vec![HotkeyBinding {
                id: 1,
                shortcut: "x".repeat(10_000)
            }]
        )
        .is_err()
    );
    assert!(
        start_wayland_hotkeys_on(
            &desktop.address,
            vec![HotkeyBinding {
                id: 99,
                shortcut: "F8".into()
            }]
        )
        .is_err()
    );
    assert!(desktop.seen.lock().unwrap().methods.is_empty());
}

#[test]
fn accepting_no_shortcuts_uses_the_fallback_instead_of_claiming_success() {
    let desktop = desktop(Answer::Empty);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| listener.status() == HotkeyStatus::Unavailable { denied: false });
    wait_for(|| desktop.seen.lock().unwrap().close_count == 1);
    assert!(!listener.take_overflow());
}

#[test]
fn revoking_all_shortcuts_releases_push_to_talk_and_uses_fallback() {
    let desktop = desktop(Answer::Accept);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(listener.status(), HotkeyStatus::Portal { .. }));
    let changed: Vec<(String, HashMap<&str, Value<'_>>)> = Vec::new();
    desktop
        .service
        .emit_signal(
            None::<&str>,
            PATH,
            GLOBAL,
            "ShortcutsChanged",
            &(ObjectPath::try_from(SESSION).unwrap(), changed),
        )
        .unwrap();
    wait_for(|| listener.status() == HotkeyStatus::Unavailable { denied: false });
    assert!(listener.take_overflow());
}

#[test]
fn desktop_rebinding_updates_status_and_releases_held_keys() {
    let desktop = desktop(Answer::Accept);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(listener.status(), HotkeyStatus::Portal { .. }));
    let changed = vec![(
        "shortcut_1",
        HashMap::from([("trigger_description", Value::from("Alt+F8"))]),
    )];
    desktop
        .service
        .emit_signal(
            None::<&str>,
            PATH,
            GLOBAL,
            "ShortcutsChanged",
            &(ObjectPath::try_from(SESSION).unwrap(), changed),
        )
        .unwrap();
    wait_for(
        || matches!(listener.status(), HotkeyStatus::Portal { ref unbound, .. } if unbound.len() == 2),
    );
    assert!(
        listener.take_overflow(),
        "rebinding must release held push-to-talk"
    );
    let HotkeyStatus::Portal { assigned, .. } = listener.status() else {
        panic!("portal stopped")
    };
    assert_eq!(assigned, [("F8".to_owned(), "Alt+F8".to_owned())]);
    emit(&desktop, SESSION, "Activated", "shortcut_16");
    emit(&desktop, SESSION, "Activated", "shortcut_1");
    let mut events = Vec::new();
    wait_for(|| {
        while let Some(event) = listener.try_next() {
            events.push(event);
        }
        !events.is_empty()
    });
    assert_eq!(
        events,
        [HotkeyEvent {
            id: 1,
            pressed: true
        }]
    );
}

#[test]
fn portal_disappearing_while_chooser_is_open_does_not_wait_for_timeout() {
    let desktop = desktop(Answer::Wait);
    let listener = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| {
        desktop
            .seen
            .lock()
            .unwrap()
            .methods
            .contains(&"bind".to_owned())
    });
    desktop
        .service
        .release_name("org.freedesktop.portal.Desktop")
        .unwrap();
    wait_for(|| listener.status() == HotkeyStatus::Unavailable { denied: false });
}

#[test]
fn starting_a_new_listener_reconnects_after_the_portal_returns() {
    let desktop = desktop(Answer::Accept);
    let first = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(first.status(), HotkeyStatus::Portal { .. }));
    desktop
        .service
        .release_name("org.freedesktop.portal.Desktop")
        .unwrap();
    wait_for(|| first.status() == HotkeyStatus::Unavailable { denied: false });
    drop(first);
    desktop
        .service
        .request_name("org.freedesktop.portal.Desktop")
        .unwrap();
    let second = start_wayland_hotkeys_on(&desktop.address, bindings()).unwrap();
    wait_for(|| matches!(second.status(), HotkeyStatus::Portal { .. }));
    emit(&desktop, SESSION, "Activated", "shortcut_1");
    wait_for(|| {
        second.try_next()
            == Some(HotkeyEvent {
                id: 1,
                pressed: true,
            })
    });
}
