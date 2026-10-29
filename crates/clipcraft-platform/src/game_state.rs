//! Opt-in loopback-only game transports. No control API, remote address, proxy or redirect.

use clipcraft_engine::game_state::{
    GameIntegration, GameIntegrationSettings, GameSnapshot, GameStateError, MAX_GAME_STATE_BYTES,
    game_state_key,
};
use clipcraft_engine::{Command, CommandOutput, execute};
use std::io::{Read, Write};
#[cfg(any(windows, target_os = "macos"))]
use std::net::SocketAddr;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FRESH: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_millis(300);
const HEADER_LIMIT: usize = 8192;

#[derive(Clone, Debug, Default)]
pub enum IntegrationStatus {
    #[default]
    Disabled,
    Waiting,
    Live(GameSnapshot),
    Error(String),
}
impl IntegrationStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Disabled => "Off".into(),
            Self::Waiting => "Waiting for game data".into(),
            Self::Live(s) => s.summary(),
            Self::Error(e) => e.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct IntegrationStatuses(pub [IntegrationStatus; 3]);
impl IntegrationStatuses {
    pub fn get(&self, game: GameIntegration) -> &IntegrationStatus {
        &self.0[index(game)]
    }
}
fn index(game: GameIntegration) -> usize {
    match game {
        GameIntegration::Cs2 => 0,
        GameIntegration::Dota2 => 1,
        GameIntegration::League => 2,
    }
}

/// One worker, one latest snapshot per game; dropping closes listeners on the worker's next turn.
pub struct GameStateWorker {
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<IntegrationStatuses>>,
    join: Option<JoinHandle<()>>,
}
impl GameStateWorker {
    pub fn start(
        settings: GameIntegrationSettings,
        key_path: PathBuf,
    ) -> Result<Self, GameStateError> {
        let latest = Arc::new(Mutex::new(IntegrationStatuses::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_latest = latest.clone();
        let worker_stop = stop.clone();
        let join = std::thread::Builder::new()
            .name("clipcraft-game-data".into())
            .spawn(move || {
                run(settings, key_path, worker_stop, worker_latest);
            })
            .map_err(|_| GameStateError::Unavailable)?;
        Ok(Self {
            stop,
            latest,
            join: Some(join),
        })
    }
    pub fn statuses(&self) -> Result<IntegrationStatuses, GameStateError> {
        if self.join.as_ref().is_some_and(|j| j.is_finished()) {
            return Err(GameStateError::Unavailable);
        }
        self.latest
            .lock()
            .map(|s| s.clone())
            .map_err(|_| GameStateError::Unavailable)
    }
    /// Ask the worker to release its sockets without blocking the UI on an in-flight read.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
}
impl Drop for GameStateWorker {
    fn drop(&mut self) {
        self.stop();
        if let Some(join) = self.join.take().filter(|j| j.is_finished()) {
            let _ = join.join();
        }
    }
}

struct ValveInput {
    listener: TcpListener,
    key: String,
    game: GameIntegration,
    last: Option<(Instant, GameSnapshot)>,
    error: Option<String>,
}
impl ValveInput {
    fn bind(game: GameIntegration, key: String, port: u16) -> Result<Self, GameStateError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .map_err(|_| GameStateError::Unavailable)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| GameStateError::Unavailable)?;
        Ok(Self {
            listener,
            key,
            game,
            last: None,
            error: None,
        })
    }
    fn poll(&mut self, stop: &AtomicBool) {
        // Never drain an unbounded connection backlog or hold a lock during IO.
        for _ in 0..4 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    match receive_gsi(&mut stream, self.game, &self.key) {
                        Ok(state) => {
                            self.last = Some((Instant::now(), state));
                            self.error = None;
                        }
                        Err(_) => {
                            self.error = Some("Rejected invalid game update. Check the config and restart the game.".into());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // A client that gave up before it was accepted is not a failed listener.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Interrupted
                            | std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                    ) => {}
                Err(_) => {
                    self.error = Some(
                        "Game data listener failed. Disable and re-enable this integration.".into(),
                    );
                    break;
                }
            }
        }
    }
    fn status(&self, now: Instant) -> IntegrationStatus {
        // Fresh game data wins over an earlier rejected request: a stray local request must not
        // hide a working integration.
        if let Some((when, state)) = &self.last
            && now.saturating_duration_since(*when) <= FRESH
        {
            return IntegrationStatus::Live(state.clone());
        }
        if let Some(e) = &self.error {
            return IntegrationStatus::Error(e.clone());
        }
        IntegrationStatus::Waiting
    }
}

fn run(
    settings: GameIntegrationSettings,
    key_path: PathBuf,
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<IntegrationStatuses>>,
) {
    let mut statuses = IntegrationStatuses::default();
    let mut inputs = Vec::new();
    for game in [GameIntegration::Cs2, GameIntegration::Dota2] {
        if settings.enabled(game) {
            match game_state_key(&key_path).and_then(|key| ValveInput::bind(game, key, game.port()))
            {
                Ok(input) => {
                    inputs.push(input);
                    statuses.0[index(game)] = IntegrationStatus::Waiting;
                }
                Err(_) => {
                    statuses.0[index(game)] = IntegrationStatus::Error(format!(
                        "Cannot open local port {} or integration key. Disable and re-enable to retry.",
                        game.port()
                    ))
                }
            }
        }
    }
    let mut league_due = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        for input in &mut inputs {
            input.poll(&stop);
            statuses.0[index(input.game)] = input.status(Instant::now());
        }
        if settings.league && Instant::now() >= league_due {
            statuses.0[2] = match poll_league() {
                Ok(state) => IntegrationStatus::Live(state),
                Err(GameStateError::Unsupported) => {
                    IntegrationStatus::Error("League's local API requires Windows or macOS.".into())
                }
                Err(_) => IntegrationStatus::Waiting,
            };
            // Slow retry while not in a game, one request/second during a match.
            league_due = Instant::now()
                + Duration::from_secs(if matches!(statuses.0[2], IntegrationStatus::Live(_)) {
                    1
                } else {
                    5
                });
        }
        if let Ok(mut output) = latest.lock() {
            *output = statuses.clone();
        } else {
            break;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

fn parse(game: GameIntegration, bytes: Vec<u8>) -> Result<GameSnapshot, GameStateError> {
    match execute(Command::ParseGameState { game, bytes }) {
        Ok(CommandOutput::GameState(state)) => Ok(*state),
        _ => Err(GameStateError::Invalid),
    }
}

fn receive_gsi(
    stream: &mut TcpStream,
    game: GameIntegration,
    key: &str,
) -> Result<GameSnapshot, GameStateError> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| GameStateError::Unavailable)?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| GameStateError::Unavailable)?;
    let result = (|| {
        let (head, body) = read_request(stream)?;
        if !head.starts_with("POST / HTTP/1.") {
            return Err(GameStateError::Invalid);
        }
        let json: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| GameStateError::Invalid)?;
        if json["auth"]["token"].as_str() != Some(key) {
            return Err(GameStateError::Invalid);
        }
        parse(game, body)
    })();
    let reply = if result.is_ok() {
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()
    } else {
        b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()
    };
    stream
        .write_all(reply)
        .map_err(|_| GameStateError::Unavailable)?;
    result
}

/// Strict HTTP input: no chunked requests, duplicate length, pipelining or unbounded headers/body.
fn read_request(reader: &mut impl Read) -> Result<(String, Vec<u8>), GameStateError> {
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(GameStateError::Invalid);
        }
        let n = reader
            .read(&mut buffer)
            .map_err(|_| GameStateError::Invalid)?;
        if n == 0 {
            return Err(GameStateError::Invalid);
        }
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            if pos > HEADER_LIMIT {
                return Err(GameStateError::Invalid);
            }
            let head = std::str::from_utf8(&bytes[..pos])
                .map_err(|_| GameStateError::Invalid)?
                .to_owned();
            let mut length = None;
            for line in head.split("\r\n").skip(1) {
                let (name, value) = line.split_once(':').ok_or(GameStateError::Invalid)?;
                if name.eq_ignore_ascii_case("transfer-encoding") {
                    return Err(GameStateError::Invalid);
                }
                if name.eq_ignore_ascii_case("content-length") {
                    if length.is_some() {
                        return Err(GameStateError::Invalid);
                    }
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| GameStateError::Invalid)?,
                    );
                }
            }
            let length = length
                .filter(|l| *l > 0 && *l <= MAX_GAME_STATE_BYTES)
                .ok_or(GameStateError::Invalid)?;
            let mut body = bytes[pos + 4..].to_vec();
            if body.len() > length {
                return Err(GameStateError::Invalid);
            }
            while body.len() < length {
                if Instant::now() >= deadline {
                    return Err(GameStateError::Invalid);
                }
                let amount = (length - body.len()).min(buffer.len());
                let n = reader
                    .read(&mut buffer[..amount])
                    .map_err(|_| GameStateError::Invalid)?;
                if n == 0 {
                    return Err(GameStateError::Invalid);
                }
                body.extend_from_slice(&buffer[..n]);
            }
            return Ok((head, body));
        }
        if bytes.len() > HEADER_LIMIT {
            return Err(GameStateError::Invalid);
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn poll_league() -> Result<GameSnapshot, GameStateError> {
    // Schannel / Security.framework, rather than a bundled C TLS/crypto implementation.
    let cert = native_tls::Certificate::from_pem(include_bytes!("riotgames.pem"))
        .map_err(|_| GameStateError::Unavailable)?;
    let connector = native_tls::TlsConnector::builder()
        .add_root_certificate(cert)
        .build()
        .map_err(|_| GameStateError::Unavailable)?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, GameIntegration::League.port()));
    let tcp =
        TcpStream::connect_timeout(&addr, IO_TIMEOUT).map_err(|_| GameStateError::Unavailable)?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| GameStateError::Unavailable)?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| GameStateError::Unavailable)?;
    // Exact local IP, normal certificate + hostname verification, no proxy/redirect/DNS/login.
    tcp.set_nonblocking(true)
        .map_err(|_| GameStateError::Unavailable)?;
    let handshake_deadline = Instant::now() + Duration::from_secs(2);
    let mut handshake = connector.connect("127.0.0.1", tcp);
    let mut tls = loop {
        match handshake {
            Ok(tls) => break tls,
            Err(native_tls::HandshakeError::WouldBlock(mid))
                if Instant::now() < handshake_deadline =>
            {
                std::thread::park_timeout(Duration::from_millis(10));
                handshake = mid.handshake();
            }
            Err(_) => return Err(GameStateError::Unavailable),
        }
    };
    tls.get_ref()
        .set_nonblocking(false)
        .map_err(|_| GameStateError::Unavailable)?;
    tls.write_all(b"GET /liveclientdata/allgamedata HTTP/1.1\r\nHost: 127.0.0.1:2999\r\nAccept: application/json\r\nConnection: close\r\n\r\n").map_err(|_| GameStateError::Unavailable)?;
    let body = read_response(&mut tls)?;
    parse(GameIntegration::League, body)
}
#[cfg(not(any(windows, target_os = "macos")))]
fn poll_league() -> Result<GameSnapshot, GameStateError> {
    Err(GameStateError::Unsupported)
}

/// Riot's local API may use either Content-Length or chunked transfer encoding.
#[cfg(any(windows, target_os = "macos", test))]
fn read_response(reader: &mut impl Read) -> Result<Vec<u8>, GameStateError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(GameStateError::Unavailable);
        }
        let n = reader
            .read(&mut buffer)
            .map_err(|_| GameStateError::Unavailable)?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_GAME_STATE_BYTES + HEADER_LIMIT {
            return Err(GameStateError::Invalid);
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    let pos = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .filter(|p| *p <= HEADER_LIMIT)
        .ok_or(GameStateError::Invalid)?;
    let head = std::str::from_utf8(&bytes[..pos]).map_err(|_| GameStateError::Invalid)?;
    let status = head.lines().next().ok_or(GameStateError::Invalid)?;
    if status != "HTTP/1.1 200 OK" && status != "HTTP/1.0 200 OK" {
        return Err(GameStateError::Unavailable);
    }
    let mut length = None;
    let mut chunked = false;
    for line in head.split("\r\n").skip(1) {
        let (name, value) = line.split_once(':').ok_or(GameStateError::Invalid)?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(GameStateError::Invalid);
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| GameStateError::Invalid)?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return Err(GameStateError::Invalid);
            }
            chunked = true;
        }
        if name.eq_ignore_ascii_case("content-encoding") && value.trim() != "identity" {
            return Err(GameStateError::Invalid);
        }
    }
    let body = &bytes[pos + 4..];
    if chunked {
        if length.is_some() {
            return Err(GameStateError::Invalid);
        }
        let mut rest = body;
        let mut output = Vec::new();
        for _ in 0..4096 {
            let end = rest
                .windows(2)
                .position(|w| w == b"\r\n")
                .filter(|n| *n <= 32)
                .ok_or(GameStateError::Invalid)?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&rest[..end]).map_err(|_| GameStateError::Invalid)?,
                16,
            )
            .map_err(|_| GameStateError::Invalid)?;
            rest = &rest[end + 2..];
            if size == 0 {
                return if rest == b"\r\n" {
                    Ok(output)
                } else {
                    Err(GameStateError::Invalid)
                };
            }
            if size > MAX_GAME_STATE_BYTES - output.len()
                || size > rest.len().saturating_sub(2)
                || &rest[size..size + 2] != b"\r\n"
            {
                return Err(GameStateError::Invalid);
            }
            output.extend_from_slice(&rest[..size]);
            rest = &rest[size + 2..];
        }
        Err(GameStateError::Invalid)
    } else {
        if body.len() > MAX_GAME_STATE_BYTES || length.is_some_and(|n| n != body.len()) {
            return Err(GameStateError::Invalid);
        }
        Ok(body.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn request(body: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn gsi_loopback_authenticates_accepts_actual_socket_and_rejects_wrong_game_or_key() {
        for (appid, key, success) in [
            (730, "test-key", true),
            (570, "test-key", false),
            (730, "wrong-key", false),
        ] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();
            assert!(addr.ip().is_loopback());
            let join = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                receive_gsi(&mut stream, GameIntegration::Cs2, "test-key")
            });
            let body = serde_json::to_vec(&json!({"provider":{"appid":appid,"steamid":"me"},"map":{"name":"de_test"},"player":{"steamid":"me","state":{"health":85}},"auth":{"token":key}})).unwrap();
            let mut client = TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            client.write_all(&request(&body)).unwrap();
            let mut reply = String::new();
            client.read_to_string(&mut reply).unwrap();
            assert_eq!(reply.contains("200 OK"), success);
            let result = join.join().unwrap();
            assert_eq!(result.is_ok(), success);
            if let Ok(s) = result {
                assert_eq!(s.health, Some(85));
                assert_eq!(s.map.as_deref(), Some("de_test"));
            }
        }
    }

    #[test]
    fn gsi_input_rejects_truncated_duplicate_chunked_and_oversized_messages() {
        let valid = request(b"{}");
        assert_eq!(read_request(&mut Cursor::new(valid)).unwrap().1, b"{}");
        for input in [
            b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}".to_vec(),
            b"POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}"
                .to_vec(),
            b"POST / HTTP/1.1\r\nContent-Length: 9999999999999999999999999999\r\n\r\n".to_vec(),
            b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}extra".to_vec(),
            vec![b'a'; HEADER_LIMIT + 10],
            format!(
                "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                MAX_GAME_STATE_BYTES + 1
            )
            .into_bytes(),
        ] {
            assert!(read_request(&mut Cursor::new(input)).is_err());
        }
    }

    #[test]
    fn stale_updates_are_never_reported_as_live_and_next_game_clears_missing_values() {
        let mut input = ValveInput::bind(GameIntegration::Dota2, "key".into(), 0).unwrap();
        let now = Instant::now();
        assert!(matches!(input.status(now), IntegrationStatus::Waiting));
        let s = parse(
            GameIntegration::Dota2,
            br#"{"provider":{"appid":570},"hero":{"health":100}}"#.to_vec(),
        )
        .unwrap();
        input.last = Some((now, s));
        assert!(matches!(input.status(now), IntegrationStatus::Live(_)));
        assert!(matches!(
            input.status(now + FRESH + Duration::from_secs(1)),
            IntegrationStatus::Waiting
        ));
        input.error = Some("invalid".into());
        // A rejected request does not hide data that is still fresh.
        assert!(matches!(input.status(now), IntegrationStatus::Live(_)));
        assert!(matches!(
            input.status(now + FRESH + Duration::from_secs(1)),
            IntegrationStatus::Error(_)
        ));
    }

    #[test]
    fn league_response_supports_length_and_chunking_but_never_redirects_or_accepts_oversize() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
            b"HTTP/1.0 200 OK\r\n\r\n{}",
        ] {
            assert_eq!(read_response(&mut Cursor::new(response)).unwrap(), b"{}");
        }
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: https://remote.example\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n{}",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{\r\n0\r\n\r\n",
        ] {
            assert!(read_response(&mut Cursor::new(response)).is_err());
        }
        assert!(
            read_response(&mut Cursor::new(vec![
                b'a';
                MAX_GAME_STATE_BYTES
                    + HEADER_LIMIT
                    + 1
            ]))
            .is_err()
        );
    }

    #[test]
    fn disabled_worker_opens_no_listeners_creates_no_key_and_stops() {
        let key = std::env::temp_dir().join(format!("unused-game-key-{}", std::process::id()));
        let worker =
            GameStateWorker::start(GameIntegrationSettings::default(), key.clone()).unwrap();
        let statuses = worker.statuses().unwrap();
        assert!(
            statuses
                .0
                .iter()
                .all(|s| matches!(s, IntegrationStatus::Disabled))
        );
        assert!(!key.exists());
        worker.stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(worker.is_finished());
    }

    #[test]
    fn a_stalled_gsi_body_times_out_without_waiting_for_game_or_ui() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let join = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            receive_gsi(&mut stream, GameIntegration::Cs2, "key")
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .write_all(b"POST / HTTP/1.1\r\nContent-Length: 100\r\n\r\n{")
            .unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.contains("400 Bad Request"));
        assert!(join.join().unwrap().is_err());
    }
}
