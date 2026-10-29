//! A small HTTPS GET for the optional update check (ADR 0032). Safe code only: the operating
//! system's own TLS (`native-tls`) does the encryption. It follows a few redirects, but only between
//! GitHub's own hosts, and it gives up on any answer larger than the caller's limit.

/// Fetches `url` with an HTTPS GET, returning the body. `limit` is the most bytes accepted.
#[cfg(any(windows, target_os = "macos"))]
pub fn https_get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        match request(&url, limit)? {
            Answer::Body(body) => return Ok(body),
            Answer::Redirect(next) => url = next,
        }
    }
    Err("too many redirects".into())
}

/// There is no system TLS to use here, so there is no update check.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn https_get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let _ = (url, limit);
    Err("update checks are not available on this system".into())
}

#[cfg(any(windows, target_os = "macos"))]
const MAX_REDIRECTS: usize = 5;

#[cfg(any(windows, target_os = "macos"))]
enum Answer {
    Body(Vec<u8>),
    Redirect(String),
}

/// Only GitHub serves releases, so only GitHub is ever contacted.
#[cfg(any(windows, target_os = "macos"))]
fn allowed_host(host: &str) -> bool {
    host == "github.com" || host == "api.github.com" || host.ends_with(".githubusercontent.com")
}

/// `https://host/path` into its host, port and path.
#[cfg(any(windows, target_os = "macos"))]
fn split(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("https://").ok_or("only https is used")?;
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(a, p)| (a, format!("/{p}")));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().map_err(|_| "bad port")?),
        None => (authority, 443),
    };
    if !allowed_host(host) || port != 443 || path.contains(['\r', '\n', ' ']) {
        return Err("this address is not one ClipCraft contacts".into());
    }
    Ok((host.to_owned(), port, path))
}

#[cfg(any(windows, target_os = "macos"))]
fn request(url: &str, limit: u64) -> Result<Answer, String> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let (host, port, path) = split(url)?;
    let address = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| "could not find the server")?
        .next()
        .ok_or("could not find the server")?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|_| "could not connect")?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(30)));
    let connector = native_tls::TlsConnector::new().map_err(|_| "no secure connection")?;
    let mut stream = connector
        .connect(&host, tcp)
        .map_err(|_| "the secure connection failed")?;
    let head = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: ClipCraft\r\nAccept: application/vnd.github+json, */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|_| "could not send the request")?;

    // Everything up to the blank line is the header; the rest is the body.
    let mut received = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    let header_end = loop {
        if let Some(at) = received.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if received.len() > 64 * 1024 {
            return Err("the answer's header was too long".into());
        }
        let read = stream
            .read(&mut chunk)
            .map_err(|_| "the connection dropped")?;
        if read == 0 {
            return Err("the connection closed early".into());
        }
        received.extend_from_slice(&chunk[..read]);
    };
    let header = String::from_utf8_lossy(&received[..header_end]).into_owned();
    let mut body = received.split_off(header_end + 4);
    let mut lines = header.lines();
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("the answer was not understood")?;
    let find = |name: &str| {
        header.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        return find("location")
            .map(Answer::Redirect)
            .ok_or_else(|| "a redirect had no address".to_owned());
    }
    if status != 200 {
        return Err(format!("the server answered {status}"));
    }
    let chunked = find("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
    // Read the rest of the body (Connection: close ends it).
    loop {
        if body.len() as u64 > limit.saturating_add(64 * 1024) {
            return Err("the answer was larger than expected".into());
        }
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            // Some servers close without a TLS goodbye once everything has been sent.
            Err(_) if !body.is_empty() => break,
            Err(_) => return Err("the connection dropped".into()),
        };
        body.extend_from_slice(&chunk[..read]);
    }
    let body = if chunked { unchunk(&body)? } else { body };
    if body.len() as u64 > limit {
        return Err("the answer was larger than expected".into());
    }
    if let Some(expected) = find("content-length").and_then(|v| v.parse::<u64>().ok())
        && !chunked
        && body.len() as u64 != expected
    {
        return Err("the download was cut short".into());
    }
    Ok(Answer::Body(body))
}

/// Joins the pieces of a `Transfer-Encoding: chunked` body.
#[cfg(any(windows, target_os = "macos"))]
fn unchunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("a chunked answer was cut short")?;
        let size_text = String::from_utf8_lossy(&data[..line_end]);
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "bad chunk size")?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if size.checked_add(2).is_none_or(|needed| data.len() < needed) {
            return Err("a chunked answer was cut short".into());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn only_github_over_https_is_contacted() {
        assert!(split("https://api.github.com/repos/a/b/releases/latest").is_ok());
        assert!(split("https://objects.githubusercontent.com/x?y=1").is_ok());
        assert!(split("http://github.com/x").is_err());
        assert!(split("https://example.com/x").is_err());
        assert!(split("https://github.com.evil.com/x").is_err());
        assert!(split("https://github.com:8443/x").is_err());
    }

    /// Needs the network, so it only runs when asked: `cargo test -p clipcraft-platform -- --ignored`.
    #[test]
    #[ignore = "needs the network"]
    fn a_release_asset_is_fetched_through_github_redirects() {
        let sums = https_get(
            "https://github.com/nanox333/clipcraft/releases/download/v0.1.0/SHA256SUMS.txt",
            64 * 1024,
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&sums).contains("clipcraft-0.1.0"));
        assert!(
            https_get(
                "https://github.com/nanox333/clipcraft/releases/download/v0.1.0/SHA256SUMS.txt",
                10
            )
            .is_err()
        );
    }

    #[test]
    fn chunked_bodies_are_joined() {
        assert_eq!(
            unchunk(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(),
            b"Wikipedia"
        );
        assert!(unchunk(b"5\r\nab").is_err());
    }
}
