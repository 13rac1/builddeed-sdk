//! A loopback HTTP server that answers NEAR RPC requests with fixed replies,
//! so that a product can test its license gating without the network.
//!
//! ```
//! use builddeed::fake_rpc;
//! let url = fake_rpc::serve(vec![fake_rpc::seat("1", "alice.testnet", true)]);
//! let product = builddeed::Product::testnet("licenses.builddeed.testnet", "checksum", "0.1.0").with_rpc_url(url);
//! let client = builddeed::Client::with_device(product, "a".repeat(64));
//! assert_eq!(client.check().unwrap().unwrap().account_id, "alice.testnet");
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// Starts the server and returns its URL. It answers one request with each
/// reply, in order: (HTTP status, JSON body).
pub fn serve(replies: Vec<(u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let url = format!("http://{}", listener.local_addr().expect("local address"));
    std::thread::spawn(move || {
        for (status, body) in replies {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let Ok(clone) = s.try_clone() else {
                return;
            };
            let mut r = BufReader::new(clone);
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body_in = vec![0u8; len];
            let _ = r.read_exact(&mut body_in);
            let _ = write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

/// A JSON-RPC reply with `result`.
pub fn ok(result: serde_json::Value) -> (u16, String) {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": "x", "result": result });
    (200, body.to_string())
}

/// A JSON-RPC error reply whose cause has this name, for example
/// `UNKNOWN_ACCOUNT`.
pub fn error(cause: &str) -> (u16, String) {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": "x",
        "error": { "name": "HANDLER_ERROR", "cause": { "name": cause, "info": {} },
                   "code": -32000, "message": "Server error" }
    });
    (200, body.to_string())
}

/// The reply of a view call that returns `value`.
pub fn view(value: serde_json::Value) -> (u16, String) {
    let bytes: Vec<u8> = value.to_string().into_bytes();
    ok(serde_json::json!({ "result": bytes, "logs": [] }))
}

/// The reply of `device_license` for a computer with a seat.
pub fn seat(token_id: &str, owner_id: &str, covers_version: bool) -> (u16, String) {
    view(serde_json::json!({
        "token_id": token_id,
        "owner_id": owner_id,
        "updates_until_ms": 1_825_882_492_821u64,
        "covers_version": covers_version,
    }))
}

/// The reply of `device_license` for a computer with no seat.
pub fn no_seat() -> (u16, String) {
    view(serde_json::Value::Null)
}
