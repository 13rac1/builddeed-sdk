//! The presence check: the register page asks the program on this computer
//! before it asks the owner's wallet.
//!
//! A register link carries a computer id. Without a check, someone could send
//! a license owner a link with their own computer id, and the owner's
//! approval would give a seat to a stranger. So while the register page is
//! open, the program answers a one-time code on 127.0.0.1:
//!
//! - `GET /builddeed/<code>` returns `{"device_id": "...", "contract": "..."}`.
//! - Only the BuildDeed website's origin may read the answer (CORS), so other
//!   websites cannot read the computer id.
//! - `OPTIONS` answers the browser's preflight, including Private Network
//!   Access (`Access-Control-Allow-Private-Network: true`).
//!
//! The page sends the owner's wallet only when the answer names the same
//! computer as the link. A link from another computer has no program behind
//! it on this one, so the check fails.

use crate::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The longest request line that the listener reads.
const MAX_LINE: u64 = 8 * 1024;

/// A running presence listener. It stops when `stop` is set or after `max`.
pub(crate) struct Presence {
    pub port: u16,
    pub code: String,
}

/// Starts a listener on 127.0.0.1 that answers `GET /builddeed/<code>` for
/// `origin` (scheme and host of the website, no path) during `max`.
pub(crate) fn start(
    device_id: &str,
    contract: &str,
    origin: &str,
    max: Duration,
    stop: Arc<AtomicBool>,
) -> Result<Presence, Error> {
    let mut raw = [0u8; 16];
    getrandom::getrandom(&mut raw).map_err(|e| Error::Io(format!("no random source: {e}")))?;
    let code = hex::encode(raw);
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| Error::Io(format!("cannot listen: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Io(e.to_string()))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Io(e.to_string()))?;
    let body = serde_json::json!({ "device_id": device_id, "contract": contract }).to_string();
    let path = format!("/builddeed/{code}");
    let origin = origin.to_string();
    std::thread::spawn(move || {
        let deadline = Instant::now() + max;
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => answer(stream, &path, &origin, &body),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => return,
            }
        }
    });
    Ok(Presence { port, code })
}

/// The scheme and host of a URL: `https://a.b/c` -> `https://a.b`.
pub(crate) fn origin_of(url: &str) -> String {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let host_end = rest.find('/').unwrap_or(rest.len());
            format!("{}{}", &url[..i + 3], &rest[..host_end])
        }
        None => url.to_string(),
    }
}

fn answer(stream: TcpStream, path: &str, origin: &str, body: &str) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut first = String::new();
    if BufReader::new((&stream).take(MAX_LINE))
        .read_line(&mut first)
        .is_err()
    {
        return;
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let cors = format!(
        "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Private-Network: true\r\nCache-Control: no-store\r\n"
    );
    let reply = match (method, target == path) {
        ("OPTIONS", _) => format!("HTTP/1.1 204 No Content\r\n{cors}Content-Length: 0\r\nConnection: close\r\n\r\n"),
        ("GET", true) => format!(
            "HTTP/1.1 200 OK\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
        _ => format!("HTTP/1.1 404 Not Found\r\n{cors}Content-Length: 0\r\nConnection: close\r\n\r\n"),
    };
    let _ = (&stream).write_all(reply.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn answers_the_code_for_the_site_origin_only() {
        let stop = Arc::new(AtomicBool::new(false));
        let p = start(
            "d1",
            "c.testnet",
            "https://13rac1.github.io",
            Duration::from_secs(10),
            stop.clone(),
        )
        .unwrap();
        assert_eq!(p.code.len(), 32);
        let ok = request(
            p.port,
            &format!("GET /builddeed/{} HTTP/1.1\r\nHost: x\r\n\r\n", p.code),
        );
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(
            ok.contains("Access-Control-Allow-Origin: https://13rac1.github.io\r\n"),
            "{ok}"
        );
        assert!(
            ok.ends_with(r#"{"contract":"c.testnet","device_id":"d1"}"#),
            "{ok}"
        );
        let wrong = request(p.port, "GET /builddeed/0000 HTTP/1.1\r\n\r\n");
        assert!(wrong.starts_with("HTTP/1.1 404"), "{wrong}");
        let pre = request(
            p.port,
            &format!("OPTIONS /builddeed/{} HTTP/1.1\r\n\r\n", p.code),
        );
        assert!(
            pre.starts_with("HTTP/1.1 204") && pre.contains("Allow-Private-Network: true"),
            "{pre}"
        );
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn stops_when_asked() {
        let stop = Arc::new(AtomicBool::new(true));
        let p = start("d", "c", "https://x", Duration::from_secs(10), stop).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // The listener is gone: the connection is refused or not answered.
        let r = TcpStream::connect(("127.0.0.1", p.port));
        if let Ok(mut s) = r {
            s.set_read_timeout(Some(Duration::from_millis(300))).ok();
            let _ = s.write_all(b"GET / HTTP/1.1\r\n\r\n");
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            assert!(out.is_empty(), "{out}");
        }
    }

    #[test]
    fn origin_is_scheme_and_host() {
        assert_eq!(
            origin_of("https://13rac1.github.io/builddeed"),
            "https://13rac1.github.io"
        );
        assert_eq!(origin_of("http://127.0.0.1:8099"), "http://127.0.0.1:8099");
    }
}
