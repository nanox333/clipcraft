//! Asking the desktop for permission to capture the screen, through the
//! `org.freedesktop.portal.ScreenCast` interface on the session bus.
//!
//! On Wayland a program may not read the screen on its own: the desktop shows its own chooser,
//! and hands back a PipeWire node and a connection to it. Nothing here is `unsafe`.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_capture::CaptureError;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREEN_CAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
/// How long a person gets to answer the desktop's chooser.
const CHOOSER_TIMEOUT: Duration = Duration::from_secs(180);
/// `SourceType`: a monitor, and a window.
const SOURCE_MONITOR: u32 = 1;
const SOURCE_WINDOW: u32 = 2;
/// `CursorMode`: not drawn, and drawn into the picture.
const CURSOR_HIDDEN: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;

type Results = HashMap<String, OwnedValue>;

/// What the desktop agreed to share.
pub struct Share {
    /// PipeWire node to receive from.
    pub node: u32,
    /// The picture size the desktop reported, when it did.
    pub size: Option<(u32, u32)>,
    /// A name for what is shared, when the desktop gave one.
    pub name: Option<String>,
    /// Connection to the PipeWire server that has the node.
    pub remote: OwnedFd,
    _session: Session,
}

/// An open portal session. Dropping it ends the share.
struct Session {
    connection: Connection,
    path: String,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(path) = ObjectPath::try_from(self.path.as_str())
            && let Ok(proxy) = Proxy::new(&self.connection, DESTINATION, path, SESSION)
        {
            let _ = proxy.call_method("Close", &());
        }
    }
}

fn unique_token(kind: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "clipcraft_{kind}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn portal(connection: &Connection) -> Result<Proxy<'static>, CaptureError> {
    Proxy::new(connection, DESTINATION, PORTAL_PATH, SCREEN_CAST)
        .map_err(|_| CaptureError::NoProvider)
}

/// The session bus, or `None` when there is none.
pub fn session_bus() -> Option<Connection> {
    Connection::session().ok()
}

/// A connection to the bus at `address`, for tests.
pub fn bus_at(address: &str) -> Option<Connection> {
    zbus::blocking::connection::Builder::address(address)
        .ok()?
        .build()
        .ok()
}

/// Whether the desktop answers screen-cast requests at all.
pub fn available() -> bool {
    session_bus()
        .and_then(|connection| {
            portal(&connection)
                .ok()?
                .get_property::<u32>("version")
                .ok()
        })
        .is_some()
}

/// Calls a portal method that answers later through a `Response` signal, and waits for the
/// answer.
fn request(
    connection: &Connection,
    screen_cast: &Proxy<'static>,
    method: &str,
    token: &str,
    arguments: impl FnOnce() -> Result<zbus::Message, zbus::Error>,
) -> Result<Results, CaptureError> {
    let _ = (screen_cast, method);
    let sender = connection
        .unique_name()
        .ok_or(CaptureError::Failed)?
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let path = ObjectPath::try_from(path).map_err(|_| CaptureError::Failed)?;
    // Listen before asking, or a fast answer is missed.
    let proxy =
        Proxy::new(connection, DESTINATION, path, REQUEST).map_err(|_| CaptureError::Failed)?;
    let mut answers = proxy
        .receive_signal("Response")
        .map_err(|_| CaptureError::Failed)?;
    arguments().map_err(|_| CaptureError::Failed)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let reply = answers
            .next()
            .and_then(|message| message.body().deserialize::<(u32, Results)>().ok());
        let _ = sender.send(reply);
    });
    let (code, results) = receiver
        .recv_timeout(CHOOSER_TIMEOUT)
        .map_err(|_| CaptureError::Failed)?
        .ok_or(CaptureError::Failed)?;
    match code {
        0 => Ok(results),
        1 => Err(CaptureError::Denied),
        _ => Err(CaptureError::Failed),
    }
}

fn text(results: &Results, key: &str) -> Option<String> {
    let value = results.get(key)?;
    String::try_from(value.try_clone().ok()?).ok().or_else(|| {
        ObjectPath::try_from(value.try_clone().ok()?)
            .ok()
            .map(|path| path.to_string())
    })
}

/// Shows the desktop's chooser and, once a screen or window is picked, returns what to receive.
pub fn start(connection: Connection, show_cursor: bool) -> Result<Share, CaptureError> {
    let screen_cast = portal(&connection)?;

    let token = unique_token("create");
    let session_token = unique_token("session");
    let created = request(&connection, &screen_cast, "CreateSession", &token, || {
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("session_handle_token", Value::from(session_token.as_str())),
        ]);
        screen_cast.call_method("CreateSession", &(options,))
    })?;
    let session = Session {
        path: text(&created, "session_handle").ok_or(CaptureError::Failed)?,
        connection: connection.clone(),
    };
    let session_path =
        ObjectPath::try_from(session.path.as_str()).map_err(|_| CaptureError::Failed)?;

    let cursors = screen_cast
        .get_property::<u32>("AvailableCursorModes")
        .unwrap_or(0);
    let cursor = if show_cursor && cursors & CURSOR_EMBEDDED != 0 {
        CURSOR_EMBEDDED
    } else {
        CURSOR_HIDDEN
    };
    let token = unique_token("select");
    request(&connection, &screen_cast, "SelectSources", &token, || {
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("types", Value::from(SOURCE_MONITOR | SOURCE_WINDOW)),
            ("multiple", Value::from(false)),
            ("cursor_mode", Value::from(cursor)),
        ]);
        screen_cast.call_method("SelectSources", &(&session_path, options))
    })?;

    let token = unique_token("start");
    let started = request(&connection, &screen_cast, "Start", &token, || {
        let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
        screen_cast.call_method("Start", &(&session_path, "", options))
    })?;
    let streams: Vec<(u32, Results)> = started
        .get("streams")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<(u32, Results)>::try_from(value).ok())
        .ok_or(CaptureError::Failed)?;
    let (node, properties) = streams.into_iter().next().ok_or(CaptureError::Failed)?;
    let size = properties
        .get("size")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| <(i32, i32)>::try_from(value).ok())
        .and_then(|(width, height)| {
            Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
        });
    let name = text(&properties, "source_type")
        .map(|kind| if kind == "2" { "Window" } else { "Monitor" }.to_owned());

    let remote: zbus::zvariant::OwnedFd = screen_cast
        .call(
            "OpenPipeWireRemote",
            &(&session_path, HashMap::<&str, Value<'_>>::new()),
        )
        .map_err(|_| CaptureError::Failed)?;
    Ok(Share {
        node,
        size,
        name,
        remote: remote.into(),
        _session: session,
    })
}
