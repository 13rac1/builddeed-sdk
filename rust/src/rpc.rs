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
            Error::Rpc(format!("the contract account does not exist ({cause})"))
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
    // HTTPS only, so that nobody on the network path can answer for the
    // contract. Loopback is allowed for tests and local nodes.
    let loopback = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| rpc_url.starts_with(p));
    if !rpc_url.starts_with("https://") && !loopback {
        return Err(Error::Rpc(format!("the RPC URL must use https: {rpc_url}")));
    }
    let resp: Value = match ureq::post(rpc_url)
        .timeout(Duration::from_secs(15))
        .send_json(body)
    {
        Ok(r) => r.into_json(),
        // nearcore answers some query errors with an HTTP error status and a
        // JSON body.
        Err(ureq::Error::Status(_, r)) => r.into_json(),
        Err(e) => return Err(Error::Network(e.to_string())),
    }
    .map_err(|e| Error::Rpc(format!("the reply is not JSON: {e}")))?;
    let result = classify(&resp)?;
    let bytes: Vec<u8> = serde_json::from_value(result["result"].clone())
        .map_err(|e| Error::Rpc(format!("unexpected view result: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| Error::Rpc(format!("unexpected view result: {e}")))
}

pub(crate) fn device_license(
    rpc_url: &str,
    contract: &str,
    device_id: &str,
    version: &str,
) -> Result<Option<DeviceLicense>, Error> {
    view(
        rpc_url,
        contract,
        "device_license",
        json!({ "device_id": device_id, "version": version }),
    )
}

pub(crate) fn find_artifact(
    rpc_url: &str,
    contract: &str,
    sha256: &str,
) -> Result<Option<ArtifactMatch>, Error> {
    view(
        rpc_url,
        contract,
        "find_artifact",
        json!({ "sha256": sha256 }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_rpc;

    #[test]
    fn errors_are_classified() {
        let url = fake_rpc::serve(vec![
            fake_rpc::error("UNKNOWN_ACCOUNT"),
            fake_rpc::error("INTERNAL_ERROR"),
            (503, "{}".into()),
        ]);
        let e = device_license(&url, "c", "d", "1").unwrap_err();
        assert!(
            matches!(&e, Error::Rpc(m) if m.contains("does not exist")),
            "{e}"
        );
        assert!(matches!(
            device_license(&url, "c", "d", "1"),
            Err(Error::Rpc(_))
        ));
        assert!(matches!(
            device_license(&url, "c", "d", "1"),
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
        let e = device_license(&format!("http://127.0.0.1:{port}"), "c", "d", "1").unwrap_err();
        assert!(matches!(e, Error::Network(_)), "{e}");
    }

    #[test]
    fn legacy_text_error_is_an_error() {
        let r = json!({ "result": { "error": "something" } });
        assert!(matches!(classify(&r), Err(Error::Rpc(_))));
    }
}
