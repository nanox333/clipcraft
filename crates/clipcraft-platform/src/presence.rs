//! Optional Discord IPC. No HTTP, login, account storage, subscription or media publication.
use clipcraft_engine::presence::PresenceTarget;
use std::io::{self, Read, Write};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 16 * 1024;
const IO_BUDGET: Duration = Duration::from_millis(500);
const UPDATE_INTERVAL: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PresenceStatus {
    #[default]
    Disabled,
    Connecting,
    Connected,
    WaitingForDiscord,
    Rejected,
    Failed,
    Clearing,
}
impl PresenceStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Off",
            Self::Connecting => "Connecting to Discord…",
            Self::Connected => "Connected to Discord",
            Self::WaitingForDiscord => "Waiting for the Discord desktop app",
            Self::Rejected => {
                "Discord rejected the activity. Check the Application ID and Discord settings."
            }
            Self::Failed => "Discord IPC failed. Disable and re-enable presence to retry.",
            Self::Clearing => "Clearing Discord activity…",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresenceError {
    Unavailable,
    Protocol,
    Rejected,
}
trait Connection: Read + Write + Send {}
impl<T: Read + Write + Send> Connection for T {}
type Connector = Box<dyn FnMut(&AtomicBool) -> Result<Box<dyn Connection>, PresenceError> + Send>;
struct Shared {
    target: Option<PresenceTarget>,
    status: PresenceStatus,
}

/// One worker and one replaceable desired state. Updating never performs IPC on the UI thread.
pub struct PresenceWorker {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl PresenceWorker {
    pub fn start() -> Result<Self, PresenceError> {
        Self::start_with(Box::new(connect), UPDATE_INTERVAL, RETRY_INTERVAL)
    }
    fn start_with(
        connector: Connector,
        interval: Duration,
        retry: Duration,
    ) -> Result<Self, PresenceError> {
        let shared = Arc::new(Mutex::new(Shared {
            target: None,
            status: PresenceStatus::Disabled,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let state = shared.clone();
        let signal = stop.clone();
        let join = std::thread::Builder::new()
            .name("clipcraft-discord".into())
            .spawn(move || run(state, signal, connector, interval, retry))
            .map_err(|_| PresenceError::Unavailable)?;
        Ok(Self {
            shared,
            stop,
            join: Some(join),
        })
    }
    pub fn publish(&self, target: Option<PresenceTarget>) -> Result<(), PresenceError> {
        let mut shared = self.shared.lock().map_err(|_| PresenceError::Unavailable)?;
        if shared.target != target {
            shared.target = target;
            self.wake();
        }
        Ok(())
    }
    pub fn status(&self) -> PresenceStatus {
        if self.join.as_ref().is_some_and(|j| j.is_finished()) {
            return PresenceStatus::Failed;
        }
        self.shared
            .lock()
            .map(|s| s.status)
            .unwrap_or(PresenceStatus::Failed)
    }
    fn wake(&self) {
        if let Some(join) = &self.join {
            join.thread().unpark();
        }
    }
    /// Bounded normal-exit cleanup. Disconnect is the fallback if Discord does not acknowledge.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake();
        let deadline = Instant::now() + Duration::from_millis(1200);
        while self.join.as_ref().is_some_and(|j| !j.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.join.as_ref().is_some_and(|j| j.is_finished())
            && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
    }
}
impl Drop for PresenceWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake();
        if self.join.as_ref().is_some_and(|j| j.is_finished())
            && let Some(join) = self.join.take()
        {
            let _ = join.join();
        }
    }
}
fn desired(shared: &Mutex<Shared>) -> Result<Option<PresenceTarget>, PresenceError> {
    shared
        .lock()
        .map(|s| s.target.clone())
        .map_err(|_| PresenceError::Unavailable)
}
fn status(shared: &Mutex<Shared>, value: PresenceStatus) {
    if let Ok(mut s) = shared.lock() {
        s.status = value;
    }
}
fn redacts(old: &PresenceTarget, new: &PresenceTarget) -> bool {
    old.disclosure & !new.disclosure != 0
        || old.activity.game != new.activity.game
        || (old.activity.match_context.is_some() && new.activity.match_context.is_none())
}

fn run(
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    mut connector: Connector,
    interval: Duration,
    retry: Duration,
) {
    let mut session: Option<Session> = None;
    let mut last: Option<PresenceTarget> = None;
    let mut changed_at: Option<Instant> = None;
    let mut retry_at = Instant::now();
    while let Ok(target) = desired(&shared) {
        if stop.load(Ordering::Relaxed) || target.is_none() {
            if let Some(mut connection) = session.take() {
                status(&shared, PresenceStatus::Clearing);
                let _ = connection.activity(None);
            }
            last = None;
            changed_at = None;
            status(&shared, PresenceStatus::Disabled);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::park_timeout(RETRY_INTERVAL);
            continue;
        }
        let Some(target) = target else {
            continue;
        };
        if session
            .as_ref()
            .is_some_and(|s| s.application_id != target.application_id)
        {
            if let Some(mut previous) = session.take() {
                status(&shared, PresenceStatus::Clearing);
                let _ = previous.activity(None);
            }
            last = None;
            changed_at = None;
            retry_at = Instant::now();
        }
        if session.is_none() && Instant::now() >= retry_at {
            status(&shared, PresenceStatus::Connecting);
            let connected = connector(&stop)
                .and_then(|stream| Session::handshake(stream, target.application_id.clone()));
            match connected {
                Ok(s) => {
                    session = Some(s);
                    last = None;
                    changed_at = None;
                }
                Err(e) => {
                    status(&shared, error_status(e));
                    retry_at = Instant::now() + retry;
                }
            }
            // Always reread consent after connecting/handshaking, before sending any activity.
            continue;
        }
        if let Some(connection) = session.as_mut() {
            let result = if last.as_ref().is_some_and(|old| redacts(old, &target)) {
                status(&shared, PresenceStatus::Clearing);
                let result = connection.activity(None);
                last = None;
                changed_at = Some(Instant::now());
                result
            } else if last
                .as_ref()
                .is_none_or(|old| old.activity != target.activity)
                && changed_at.is_none_or(|at| at.elapsed() >= interval)
            {
                let result = connection.activity(Some(&target));
                if result.is_ok() {
                    last = Some(target);
                    changed_at = Some(Instant::now());
                    status(&shared, PresenceStatus::Connected);
                }
                result
            } else {
                connection.idle()
            };
            if let Err(e) = result {
                session = None;
                last = None;
                changed_at = None;
                retry_at = Instant::now() + retry;
                status(&shared, error_status(e));
            }
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}
fn error_status(error: PresenceError) -> PresenceStatus {
    match error {
        PresenceError::Rejected => PresenceStatus::Rejected,
        _ => PresenceStatus::WaitingForDiscord,
    }
}

struct Session {
    stream: Box<dyn Connection>,
    incoming: Vec<u8>,
    nonce: u64,
    application_id: String,
}
impl Session {
    fn handshake(
        stream: Box<dyn Connection>,
        application_id: String,
    ) -> Result<Self, PresenceError> {
        let mut s = Self {
            stream,
            incoming: Vec::new(),
            nonce: 0,
            application_id,
        };
        let bytes = serde_json::to_vec(&serde_json::json!({"v":1,"client_id":s.application_id}))
            .map_err(|_| PresenceError::Protocol)?;
        s.send(0, &bytes, Instant::now() + IO_BUDGET)?;
        s.wait(None)?;
        Ok(s)
    }
    fn activity(&mut self, target: Option<&PresenceTarget>) -> Result<(), PresenceError> {
        self.nonce = self.nonce.checked_add(1).ok_or(PresenceError::Protocol)?;
        let nonce = self.nonce.to_string();
        let activity = target
            .map(|t| t.activity.wire())
            .unwrap_or(serde_json::Value::Null);
        let bytes=serde_json::to_vec(&serde_json::json!({"cmd":"SET_ACTIVITY","args":{"pid":std::process::id(),"activity":activity},"nonce":nonce})).map_err(|_|PresenceError::Protocol)?;
        self.send(1, &bytes, Instant::now() + IO_BUDGET)?;
        self.wait(Some(&nonce))
    }
    fn send(
        &mut self,
        opcode: u32,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<(), PresenceError> {
        if payload.len() > MAX_FRAME {
            return Err(PresenceError::Protocol);
        }
        let mut bytes = Vec::with_capacity(payload.len() + 8);
        bytes.extend_from_slice(&opcode.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        let mut offset = 0;
        while offset < bytes.len() {
            if Instant::now() >= deadline {
                return Err(PresenceError::Unavailable);
            }
            match self.stream.write(&bytes[offset..]) {
                Ok(0) => return Err(PresenceError::Unavailable),
                Ok(n) => offset += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::park_timeout(Duration::from_millis(5))
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(PresenceError::Unavailable),
            }
        }
        Ok(())
    }
    fn frame(&mut self) -> Result<Option<(u32, Vec<u8>)>, PresenceError> {
        let mut buffer = [0u8; 4096];
        for _ in 0..16 {
            if self.incoming.len() >= 8 {
                let op = u32::from_le_bytes(
                    self.incoming[..4]
                        .try_into()
                        .map_err(|_| PresenceError::Protocol)?,
                );
                let size = u32::from_le_bytes(
                    self.incoming[4..8]
                        .try_into()
                        .map_err(|_| PresenceError::Protocol)?,
                ) as usize;
                if size > MAX_FRAME || op > 4 {
                    return Err(PresenceError::Protocol);
                }
                if self.incoming.len() >= size + 8 {
                    let body = self.incoming[8..size + 8].to_vec();
                    self.incoming.drain(..size + 8);
                    return Ok(Some((op, body)));
                }
            }
            let amount = (MAX_FRAME + 8 - self.incoming.len()).min(buffer.len());
            match self.stream.read(&mut buffer[..amount]) {
                Ok(0) => return Err(PresenceError::Unavailable),
                Ok(n) => self.incoming.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return Err(PresenceError::Unavailable),
            }
        }
        Ok(None)
    }
    fn wait(&mut self, nonce: Option<&str>) -> Result<(), PresenceError> {
        let deadline = Instant::now() + IO_BUDGET;
        while Instant::now() < deadline {
            let Some((op, bytes)) = self.frame()? else {
                std::thread::park_timeout(Duration::from_millis(5));
                continue;
            };
            match op {
                3 => self.send(4, &bytes, deadline)?,
                2 => return Err(PresenceError::Unavailable),
                1 => {
                    let v: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| PresenceError::Protocol)?;
                    if v["evt"] == "ERROR" {
                        return Err(PresenceError::Rejected);
                    }
                    if nonce.is_none() && v["cmd"] == "DISPATCH" && v["evt"] == "READY" {
                        return Ok(());
                    }
                    if let Some(nonce) = nonce
                        && v["cmd"] == "SET_ACTIVITY"
                        && v["nonce"].as_str() == Some(nonce)
                    {
                        return Ok(());
                    }
                }
                4 => {}
                _ => return Err(PresenceError::Protocol),
            }
        }
        Err(PresenceError::Unavailable)
    }
    fn idle(&mut self) -> Result<(), PresenceError> {
        let deadline = Instant::now() + IO_BUDGET;
        for _ in 0..8 {
            let Some((op, bytes)) = self.frame()? else {
                break;
            };
            match op {
                3 => self.send(4, &bytes, deadline)?,
                4 => {}
                2 => return Err(PresenceError::Unavailable),
                1 => {
                    let v: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| PresenceError::Protocol)?;
                    if v["evt"] == "ERROR" {
                        return Err(PresenceError::Rejected);
                    }
                }
                _ => return Err(PresenceError::Protocol),
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn connect(stop: &AtomicBool) -> Result<Box<dyn Connection>, PresenceError> {
    use std::os::unix::fs::FileTypeExt;
    let mut roots = Vec::new();
    for name in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Some(root) = std::env::var_os(name).filter(|s| s.len() <= 240) {
            let root = std::path::PathBuf::from(root);
            if root.is_absolute() && !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    if !roots.contains(&std::path::PathBuf::from("/tmp")) {
        roots.push("/tmp".into());
    }
    for root in roots {
        for directory in [
            &root,
            &root.join("app/com.discordapp.Discord"),
            &root.join("snap.discord"),
        ] {
            for index in 0..10 {
                if stop.load(Ordering::Relaxed) {
                    return Err(PresenceError::Unavailable);
                }
                let path = directory.join(format!("discord-ipc-{index}"));
                if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_socket()) {
                    continue;
                }
                let Ok(addr) = socket2::SockAddr::unix(&path) else {
                    continue;
                };
                let Ok(socket) =
                    socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)
                else {
                    continue;
                };
                if socket
                    .connect_timeout(&addr, Duration::from_millis(10))
                    .is_ok()
                    && socket.set_nonblocking(true).is_ok()
                {
                    let fd: std::os::fd::OwnedFd = socket.into();
                    let stream = std::os::unix::net::UnixStream::from(fd);
                    return Ok(Box::new(stream));
                }
            }
        }
    }
    Err(PresenceError::Unavailable)
}

#[cfg(windows)]
struct WindowsPipe(
    interprocess::os::windows::named_pipe::DuplexPipeStream<
        interprocess::os::windows::named_pipe::pipe_mode::Bytes,
    >,
);
#[cfg(windows)]
impl Read for WindowsPipe {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
#[cfg(windows)]
impl Write for WindowsPipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for WindowsPipe {
    fn drop(&mut self) {
        self.0.assume_flushed();
    }
}
#[cfg(windows)]
fn connect(stop: &AtomicBool) -> Result<Box<dyn Connection>, PresenceError> {
    for index in 0..10 {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        if let Ok(pipe) =
            interprocess::os::windows::named_pipe::DuplexPipeStream::connect_by_path_with_wait_mode(
                path,
                interprocess::ConnectWaitMode::Timeout(Duration::ZERO),
            )
        {
            if pipe.set_nonblocking(true).is_ok() {
                return Ok(Box::new(WindowsPipe(pipe)));
            }
            pipe.assume_flushed();
        }
    }
    Err(PresenceError::Unavailable)
}
#[cfg(not(any(unix, windows)))]
fn connect(_: &AtomicBool) -> Result<Box<dyn Connection>, PresenceError> {
    Err(PresenceError::Unavailable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;

    fn pair() -> (Box<dyn Connection>, UnixStream) {
        let (client, server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        (Box::new(client), server)
    }
    fn read(stream: &mut UnixStream) -> (u32, Vec<u8>) {
        let mut head = [0; 8];
        stream.read_exact(&mut head).unwrap();
        let op = u32::from_le_bytes(head[..4].try_into().unwrap());
        let n = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        assert!(n <= MAX_FRAME);
        let mut body = vec![0; n];
        stream.read_exact(&mut body).unwrap();
        (op, body)
    }
    fn write(stream: &mut UnixStream, op: u32, value: Value) {
        let body = serde_json::to_vec(&value).unwrap();
        let mut frame = Vec::new();
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(&body);
        stream.write_all(&frame).unwrap();
    }
    fn ready(stream: &mut UnixStream) {
        let (op, body) = read(stream);
        assert_eq!(op, 0);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v, json!({"v":1,"client_id":"123456789012345678"}));
        write(
            stream,
            1,
            json!({"cmd":"DISPATCH","evt":"READY","data":{"user":{"id":"private-account","username":"private-name"}}}),
        );
    }
    fn ack(stream: &mut UnixStream) -> Value {
        let (op, body) = read(stream);
        assert_eq!(op, 1);
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        write(
            stream,
            1,
            json!({"cmd":"SET_ACTIVITY","nonce":v["nonce"],"data":null}),
        );
        v
    }
    fn target(state: Option<&str>) -> PresenceTarget {
        PresenceTarget {
            application_id: "123456789012345678".into(),
            disclosure: if state.is_some() { 3 } else { 0 },
            activity: clipcraft_engine::presence::PresenceActivity {
                details: "Clipping".into(),
                game: state.map(|_| "Counter-Strike 2".into()),
                match_context: state.map(str::to_owned),
            },
        }
    }
    fn until(mut condition: impl FnMut() -> bool) {
        let end = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < end, "worker did not reach expected state");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn actual_ipc_handshake_activity_ping_ack_and_clear_discard_account_data() {
        let (client, mut server) = pair();
        let join = std::thread::spawn(move || {
            ready(&mut server);
            let (op, bytes) = read(&mut server);
            assert_eq!(op, 1);
            let activity: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                activity["args"]["activity"]["state"],
                "Counter-Strike 2 · Mirage · 8–6"
            );
            assert!(!String::from_utf8_lossy(&bytes).contains("private"));
            write(&mut server, 3, json!({"ping":"echo"}));
            let (op, pong) = read(&mut server);
            assert_eq!(op, 4);
            assert_eq!(
                serde_json::from_slice::<Value>(&pong).unwrap(),
                json!({"ping":"echo"})
            );
            write(
                &mut server,
                1,
                json!({"cmd":"SET_ACTIVITY","nonce":activity["nonce"],"data":{}}),
            );
            assert!(ack(&mut server)["args"]["activity"].is_null());
        });
        let mut session = Session::handshake(client, "123456789012345678".into()).unwrap();
        session
            .activity(Some(&target(Some("Mirage · 8–6"))))
            .unwrap();
        session.activity(None).unwrap();
        join.join().unwrap();
    }

    #[test]
    fn malformed_oversized_truncated_closed_and_rejected_peers_fail_boundedly() {
        for (op, body) in [
            (1, b"{".to_vec()),
            (
                1,
                serde_json::to_vec(&json!({"evt":"ERROR","data":{"message":"private"}})).unwrap(),
            ),
            (2, b"{}".to_vec()),
            (88, b"{}".to_vec()),
        ] {
            let (client, mut server) = pair();
            let join = std::thread::spawn(move || {
                let _ = read(&mut server);
                let mut frame = Vec::new();
                frame.extend_from_slice(&(op as u32).to_le_bytes());
                frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
                frame.extend_from_slice(&body);
                server.write_all(&frame).unwrap();
            });
            assert!(Session::handshake(client, "123456789012345678".into()).is_err());
            join.join().unwrap();
        }
        for size in [MAX_FRAME as u32 + 1, u32::MAX] {
            let (client, mut server) = pair();
            let join = std::thread::spawn(move || {
                let _ = read(&mut server);
                server.write_all(&[1, 0, 0, 0]).unwrap();
                server.write_all(&size.to_le_bytes()).unwrap();
            });
            assert!(Session::handshake(client, "123456789012345678".into()).is_err());
            join.join().unwrap();
        }
        let (client, mut server) = pair();
        let join = std::thread::spawn(move || {
            let _ = read(&mut server);
            server.write_all(&[1, 0]).unwrap();
        });
        assert!(Session::handshake(client, "123456789012345678".into()).is_err());
        join.join().unwrap();
    }

    #[test]
    fn idle_peer_and_wrong_nonce_cannot_hang_or_fake_an_acknowledgement() {
        let (client, mut server) = pair();
        let join = std::thread::spawn(move || {
            ready(&mut server);
            let _ = read(&mut server);
            write(
                &mut server,
                1,
                json!({"cmd":"SET_ACTIVITY","nonce":"wrong"}),
            );
            std::thread::sleep(IO_BUDGET + Duration::from_millis(100));
        });
        let mut session = Session::handshake(client, "123456789012345678".into()).unwrap();
        let started = Instant::now();
        assert!(session.activity(Some(&target(None))).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        join.join().unwrap();
    }

    #[test]
    fn disabled_worker_never_discovers_discord() {
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = probes.clone();
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
                Err(PresenceError::Unavailable)
            }),
            Duration::from_millis(100),
            Duration::from_millis(20),
        )
        .unwrap();
        worker.publish(None).unwrap();
        worker.shutdown();
        assert_eq!(probes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn expanding_consent_without_changing_activity_sends_no_redundant_update() {
        let (client, mut server) = pair();
        let mut client = Some(client);
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            ready(&mut server);
            tx.send(ack(&mut server)).unwrap();
            tx.send(ack(&mut server)).unwrap();
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| client.take().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(20),
            Duration::from_millis(20),
        )
        .unwrap();
        worker.publish(Some(target(None))).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut same = target(None);
        same.disclosure = 3;
        worker.publish(Some(same)).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(180)).is_err());
        worker.publish(None).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap()["args"]["activity"].is_null());
        worker.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn latest_state_is_coalesced_unchanged_updates_are_suppressed_and_disable_clears() {
        let (client, mut server) = pair();
        let mut client = Some(client);
        let (tx, rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            ready(&mut server);
            tx.send(ack(&mut server)).unwrap();
            tx.send(ack(&mut server)).unwrap();
            tx.send(ack(&mut server)).unwrap();
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| client.take().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(180),
            Duration::from_millis(20),
        )
        .unwrap();
        worker.publish(Some(target(Some("first")))).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap()["args"]["activity"]["state"],
            "Counter-Strike 2 · first"
        );
        for i in 0..100 {
            worker
                .publish(Some(target(Some(&format!("update {i}")))))
                .unwrap();
        }
        let last = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            last["args"]["activity"]["state"],
            "Counter-Strike 2 · update 99"
        );
        for _ in 0..20 {
            worker.publish(Some(target(Some("update 99")))).unwrap();
        }
        assert!(rx.recv_timeout(Duration::from_millis(230)).is_err());
        worker.publish(None).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(3)).unwrap()["args"]["activity"].is_null());
        until(|| worker.status() == PresenceStatus::Disabled);
        worker.shutdown();
        join.join().unwrap();
    }

    #[test]
    fn redactions_clear_immediately_even_during_update_rate_limit_and_exit_clears_again() {
        let (client, mut server) = pair();
        let mut client = Some(client);
        let (tx, rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            ready(&mut server);
            for _ in 0..4 {
                tx.send(ack(&mut server)).unwrap();
            }
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| client.take().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(200),
            Duration::from_millis(20),
        )
        .unwrap();
        worker
            .publish(Some(target(Some("private old match"))))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.publish(Some(target(None))).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(180)).unwrap()["args"]["activity"].is_null());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap()["args"]["activity"],
            json!({"details":"Clipping"})
        );
        worker.shutdown();
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap()["args"]["activity"].is_null());
        join.join().unwrap();
    }

    #[test]
    fn disconnect_reconnect_does_not_reuse_old_activity() {
        let (first, mut server1) = pair();
        let (second, mut server2) = pair();
        let mut clients = std::collections::VecDeque::from([first, second]);
        let (tx, rx) = mpsc::channel();
        let tx1 = tx.clone();
        let a = std::thread::spawn(move || {
            ready(&mut server1);
            tx1.send(ack(&mut server1)).unwrap();
        });
        let b = std::thread::spawn(move || {
            ready(&mut server2);
            tx.send(ack(&mut server2)).unwrap();
            tx.send(ack(&mut server2)).unwrap();
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| clients.pop_front().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(50),
            Duration::from_millis(20),
        )
        .unwrap();
        worker
            .publish(Some(target(Some("before reconnect"))))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.publish(Some(target(None))).unwrap();
        let restored = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(restored["args"]["activity"], json!({"details":"Clipping"}));
        worker.shutdown();
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap()["args"]["activity"].is_null());
        a.join().unwrap();
        b.join().unwrap();
    }

    #[test]
    fn opting_out_during_handshake_never_publishes_the_queued_match() {
        let (client, mut server) = pair();
        let mut client = Some(client);
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (op, _) = read(&mut server);
            assert_eq!(op, 0);
            arrived_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            write(&mut server, 1, json!({"cmd":"DISPATCH","evt":"READY"}));
            assert!(ack(&mut server)["args"]["activity"].is_null());
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| client.take().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(20),
            Duration::from_millis(20),
        )
        .unwrap();
        worker
            .publish(Some(target(Some("must never be published"))))
            .unwrap();
        arrived_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.publish(None).unwrap();
        release_tx.send(()).unwrap();
        until(|| worker.status() == PresenceStatus::Disabled);
        worker.shutdown();
        server.join().unwrap();
    }

    #[test]
    fn changing_application_id_clears_old_session_before_handshaking_the_new_one() {
        let (first, mut server1) = pair();
        let (second, mut server2) = pair();
        let mut clients = std::collections::VecDeque::from([first, second]);
        let (tx, rx) = mpsc::channel();
        let tx1 = tx.clone();
        let a = std::thread::spawn(move || {
            ready(&mut server1);
            tx1.send(ack(&mut server1)).unwrap();
            assert!(ack(&mut server1)["args"]["activity"].is_null());
        });
        let b = std::thread::spawn(move || {
            let (op, bytes) = read(&mut server2);
            assert_eq!(op, 0);
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["client_id"],
                "123456789012345679"
            );
            write(&mut server2, 1, json!({"cmd":"DISPATCH","evt":"READY"}));
            tx.send(ack(&mut server2)).unwrap();
            assert!(ack(&mut server2)["args"]["activity"].is_null());
        });
        let mut worker = PresenceWorker::start_with(
            Box::new(move |_| clients.pop_front().ok_or(PresenceError::Unavailable)),
            Duration::from_millis(20),
            Duration::from_millis(20),
        )
        .unwrap();
        worker.publish(Some(target(Some("old match")))).unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut next = target(None);
        next.application_id = "123456789012345679".into();
        worker.publish(Some(next)).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap()["args"]["activity"],
            json!({"details":"Clipping"})
        );
        worker.shutdown();
        a.join().unwrap();
        b.join().unwrap();
    }
}
