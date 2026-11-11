//! View calls to a BuildDeed license contract through NEAR JSON-RPC.

use crate::{ArtifactMatch, Error};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

/// The answer of the contract's `device_license` view.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct DeviceLicense {
    pub token_id: String,
    pub owner_id: String,
    pub updates_until_ms: u64,
    pub covers_version: bool,
}

/// The names that nearcore gives to "does not exist" errors.
const UNKNOWN_CAUSES: [&str; 2] = ["UNKNOWN_ACCESS_KEY", "UNKNOWN_ACCOUNT"];

/// Classifies an RPC reply: the result, or the error by its cause name.
fn classify(resp: &Value) -> Result<Value, Error> {
    if let Some(err) = resp.get("error") {
        let cause = err
            .pointer("/cause/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(if UNKNOWN_CAUSES.contains(&cause) {
            Error::Rpc(format!(
                "the account or the access key does not exist ({cause})"
            ))
        } else {
            Error::Rpc(err.to_string())
        });
    }
    let result = resp
        .get("result")
        .cloned()
        .ok_or_else(|| Error::Rpc("the reply has no result".into()))?;
    // Older nodes put a query error in result.error as text.
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        return Err(Error::Rpc(err.to_string()));
    }
    Ok(result)
}

pub(crate) fn view<T: serde::de::DeserializeOwned>(
    rpc_url: &str,
    contract: &str,
    method: &str,
    args: Value,
) -> Result<T, Error> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": "builddeed",
        "method": "query",
        "params": {
            "request_type": "call_function",
            "finality": "final",
            "account_id": contract,
            "method_name": method,
            "args_base64": B64.encode(args.to_string()),
        }
    });
    let result = post(rpc_url, body)?;
    let bytes: Vec<u8> = serde_json::from_value(result["result"].clone())
        .map_err(|e| Error::Rpc(format!("unexpected view result: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::Rpc(format!("unexpected view result: {e}")))
}

/// Sends one JSON-RPC request and returns its classified result.
pub(crate) fn post(rpc_url: &str, body: Value) -> Result<Value, Error> {
    let (_, text) = post_raw(rpc_url, &body)?;
    let resp: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Rpc(format!("the reply is not JSON: {e}")))?;
    classify(&resp)
}

/// Sends one JSON-RPC request and returns the HTTP status and the body as
/// text. nearcore answers some errors with an HTTP error status and a JSON
/// body, so an error status is not an error here.
pub(crate) fn post_raw(rpc_url: &str, body: &Value) -> Result<(u16, String), Error> {
    if !url_allowed(rpc_url) {
        return Err(Error::Rpc(format!(
            "the RPC URL must use https (or http to a loopback host): {rpc_url}"
        )));
    }
    let resp = match ureq::post(rpc_url)
        .timeout(Duration::from_secs(30))
        .send_json(body)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(Error::Network(e.to_string())),
    };
    let status = resp.status();
    let text = resp
        .into_string()
        .map_err(|e| Error::Network(e.to_string()))?;
    Ok((status, text))
}

/// HTTPS only, so that nobody on the network path can answer for the
/// contract. Plain http is allowed only to a loopback host (tests, local
/// nodes), compared as the parsed host name: `http://127.0.0.1.evil.example`
/// is not loopback.
pub(crate) fn url_allowed(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, port)) if port.is_empty() || port.starts_with(':') => h,
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or("")
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub(crate) fn device_license(
    rpc_url: &str,
    contract: &str,
    product: &str,
    device_id: &str,
    version: &str,
) -> Result<Option<DeviceLicense>, Error> {
    view(
        rpc_url,
        contract,
        "device_license",
        json!({ "product": product, "device_id": device_id, "version": version }),
    )
}

pub(crate) fn find_artifact(
    rpc_url: &str,
    contract: &str,
    product: &str,
    sha256: &str,
) -> Result<Option<ArtifactMatch>, Error> {
    view(
        rpc_url,
        contract,
        "find_artifact",
        json!({ "product": product, "sha256": sha256 }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_url_vectors() {
        let f: Value = serde_json::from_str(include_str!("../../fixtures/rpc-urls.json")).unwrap();
        for u in f["accept"].as_array().unwrap() {
            assert!(url_allowed(u.as_str().unwrap()), "{u}");
        }
        for u in f["refuse"].as_array().unwrap() {
            assert!(!url_allowed(u.as_str().unwrap()), "{u}");
        }
    }
    use crate::fake_rpc;

    #[test]
    fn errors_are_classified() {
        let url = fake_rpc::serve(vec![
            fake_rpc::error("UNKNOWN_ACCOUNT"),
            fake_rpc::error("INTERNAL_ERROR"),
            (503, "{}".into()),
        ]);
        let e = device_license(&url, "c", "p", "d", "1").unwrap_err();
        assert!(
            matches!(&e, Error::Rpc(m) if m.contains("does not exist")),
            "{e}"
        );
        assert!(matches!(
            device_license(&url, "c", "p", "d", "1"),
            Err(Error::Rpc(_))
        ));
        assert!(matches!(
            device_license(&url, "c", "p", "d", "1"),
            Err(Error::Rpc(_))
        ));
    }

    #[test]
    fn no_network_is_a_network_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let e =
            device_license(&format!("http://127.0.0.1:{port}"), "c", "p", "d", "1").unwrap_err();
        assert!(matches!(e, Error::Network(_)), "{e}");
    }

    #[test]
    fn legacy_text_error_is_an_error() {
        let r = json!({ "result": { "error": "something" } });
        assert!(matches!(classify(&r), Err(Error::Rpc(_))));
    }
}
