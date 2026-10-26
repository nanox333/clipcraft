//! Asking the desktop for permission to capture the screen, through the
//! `org.freedesktop.portal.ScreenCast` interface on the session bus.
//!
//! On Wayland a program may not read the screen on its own: the desktop shows its own chooser,
//! and hands back a PipeWire node and a connection to it. Nothing here is `unsafe`.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
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

/// `PersistMode`: keep the choice until the person revokes it.
const PERSIST_UNTIL_REVOKED: u32 = 2;
/// The portal version that can remember a choice (`restore_token`).
const RESTORE_VERSION: u32 = 4;
/// Longest saved token read back; real ones are short strings.
const MAX_TOKEN: usize = 256;

type Results = HashMap<String, OwnedValue>;

/// The saved restore token, if there is a sane one. A token lets the desktop skip its chooser and
/// share the screen or window the person picked last time.
pub fn read_token(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let token = text.trim();
    (!token.is_empty() && token.len() <= MAX_TOKEN && token.chars().all(|c| c.is_ascii_graphic()))
        .then(|| token.to_owned())
}

/// Saves the token the desktop handed back, replacing the old one atomically and readable only by
/// this user. Failure to save only means the chooser shows next time.
pub fn write_token(path: &Path, token: &str) {
    if token.is_empty() || token.len() > MAX_TOKEN {
        return;
    }
    let temporary = path.with_extension("partial");
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(token.as_bytes()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// Forgets the saved choice, so the desktop asks again.
pub fn forget_token(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// The extra `SelectSources` options that ask the desktop to remember the choice, and to reuse
/// the one saved earlier. Older portals do not know them, so they get none.
fn remember_options(version: u32, saved: Option<&str>) -> Vec<(&'static str, Value<'static>)> {
    if version < RESTORE_VERSION {
        return Vec::new();
    }
    let mut options = vec![("persist_mode", Value::from(PERSIST_UNTIL_REVOKED))];
    if let Some(token) = saved {
        options.push(("restore_token", Value::from(token.to_owned())));
    }
    options
}

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
/// With `remember`, the choice is saved in that file and reused next time, so the chooser only
/// shows once.
pub fn start(
    connection: Connection,
    show_cursor: bool,
    remember: Option<&Path>,
) -> Result<Share, CaptureError> {
    let screen_cast = portal(&connection)?;
    let version = screen_cast.get_property::<u32>("version").unwrap_or(0);
    let saved = remember.and_then(read_token);

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
        let mut options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("types", Value::from(SOURCE_MONITOR | SOURCE_WINDOW)),
            ("multiple", Value::from(false)),
            ("cursor_mode", Value::from(cursor)),
        ]);
        if remember.is_some() {
            options.extend(remember_options(version, saved.as_deref()));
        }
        screen_cast.call_method("SelectSources", &(&session_path, options))
    })?;

    let token = unique_token("start");
    let started = request(&connection, &screen_cast, "Start", &token, || {
        let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
        screen_cast.call_method("Start", &(&session_path, "", options))
    })?;
    if let (Some(path), Some(token)) = (remember, text(&started, "restore_token")) {
        write_token(path, &token);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-portal-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    #[test]
    fn a_saved_choice_round_trips_privately_and_bad_files_are_ignored() {
        let dir = scratch("token");
        let path = dir.join("screencast-restore-token");
        assert_eq!(read_token(&path), None);
        write_token(&path, "abc-123_XYZ");
        assert_eq!(read_token(&path).as_deref(), Some("abc-123_XYZ"));
        // Saving again replaces it, and only this user may read the file.
        write_token(&path, "second");
        assert_eq!(read_token(&path).as_deref(), Some("second"));
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("file").permissions().mode();
            assert_eq!(mode & 0o077, 0, "mode {mode:o}");
        }
        assert!(!dir.join("screencast-restore-token.partial").exists());
        // Nonsense is never sent to the desktop.
        std::fs::write(&path, "has a space").expect("write");
        assert_eq!(read_token(&path), None);
        std::fs::write(&path, "x".repeat(MAX_TOKEN + 1)).expect("write");
        assert_eq!(read_token(&path), None);
        std::fs::write(&path, "").expect("write");
        assert_eq!(read_token(&path), None);
        write_token(&path, "");
        write_token(&path, &"y".repeat(MAX_TOKEN + 1));
        assert_eq!(read_token(&path), None);
        write_token(&path, "again");
        forget_token(&path);
        assert_eq!(read_token(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_portal_that_can_remember_is_asked_to() {
        assert!(remember_options(3, Some("t")).is_empty());
        assert!(remember_options(0, None).is_empty());
        let first = remember_options(4, None);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, "persist_mode");
        let again = remember_options(5, Some("t"));
        assert_eq!(again.len(), 2);
        assert_eq!(again[1].0, "restore_token");
    }
}
