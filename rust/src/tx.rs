//! Signed function calls, for programs and agents that have no browser.
//!
//! An owner gives an agent a NEAR function-call access key. Such a key can
//! call only the methods that the owner lists, on one contract, and it
//! cannot attach NEAR. The BuildDeed contract has two methods for it,
//! `buy_from_budget` and `add_device_from_budget`, which pay from a budget
//! that the owner deposited with a full-access key.
//!
//! The transaction format is the borsh encoding of a NEAR `Transaction` with
//! one `FunctionCall` action, signed with ed25519 over its SHA-256.

use crate::rpc;
use crate::Error;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Gas for one call: enough for a purchase with its transfers.
const GAS: u64 = 100_000_000_000_000;

/// A NEAR key file, as near-cli writes it:
/// `{"account_id": "...", "public_key": "ed25519:...", "private_key": "ed25519:..."}`.
/// A key file in `~/.near-credentials/<network>/<account>/` has no
/// `account_id`; [`KeyFile::load`] then takes the account from the folder
/// name.
#[derive(Clone, Deserialize)]
pub struct KeyFile {
    /// The account that the key belongs to.
    #[serde(default)]
    pub account_id: String,
    /// `ed25519:` and the base58 public key.
    pub public_key: String,
    private_key: String,
}

impl std::fmt::Debug for KeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the private key.
        f.debug_struct("KeyFile")
            .field("account_id", &self.account_id)
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

impl KeyFile {
    /// Reads a key file.
    pub fn load(path: &std::path::Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(format!("cannot read {}: {e}", path.display())))?;
        let mut k = Self::parse(&text)?;
        if k.account_id.is_empty() {
            // Empty when the folder is not an account: the caller gives it
            // with with_account, or the call refuses the key.
            if let Some(name) = path
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .filter(|n| n.contains('.') || n.len() == 64)
            {
                k.account_id = name.to_string();
            }
        }
        Ok(k)
    }

    /// The same key for another account.
    pub fn with_account(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = account_id.into();
        self
    }

    /// Parses the JSON of a key file and checks that the private key matches
    /// the public key.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let k: KeyFile = serde_json::from_str(json)
            .map_err(|e| Error::Io(format!("not a NEAR key file: {e}")))?;
        let signing = k.signing_key()?;
        let public = decode_key(&k.public_key, 32)?;
        if signing.verifying_key().as_bytes()[..] != public[..] {
            return Err(Error::Io(
                "the private key of the key file does not match its public key".into(),
            ));
        }
        Ok(k)
    }

    fn signing_key(&self) -> Result<SigningKey, Error> {
        // near-cli writes the 64-byte form: the secret, then the public key.
        let raw = decode_key(&self.private_key, 64)?;
        let secret: [u8; 32] = raw[..32].try_into().expect("64 bytes checked");
        Ok(SigningKey::from_bytes(&secret))
    }
}

/// Decodes `ed25519:<base58>` to `len` bytes.
fn decode_key(text: &str, len: usize) -> Result<Vec<u8>, Error> {
    let b58 = text
        .strip_prefix("ed25519:")
        .ok_or_else(|| Error::Io("only ed25519 keys are supported".into()))?;
    let raw = bs58::decode(b58)
        .into_vec()
        .map_err(|e| Error::Io(format!("bad base58 key: {e}")))?;
    if raw.len() != len {
        return Err(Error::Io(format!(
            "an ed25519 key has {len} bytes here, not {}",
            raw.len()
        )));
    }
    Ok(raw)
}

/// The borsh bytes of a `Transaction` with one `FunctionCall` action and no
/// deposit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_call(
    signer: &str,
    public_key: &[u8; 32],
    nonce: u64,
    receiver: &str,
    block_hash: &[u8; 32],
    method: &str,
    args: &[u8],
    gas: u64,
) -> Vec<u8> {
    let mut out = Vec::new();
    put_str(&mut out, signer);
    out.push(0); // KeyType::ED25519
    out.extend_from_slice(public_key);
    out.extend_from_slice(&nonce.to_le_bytes());
    put_str(&mut out, receiver);
    out.extend_from_slice(block_hash);
    out.extend_from_slice(&1u32.to_le_bytes()); // one action
    out.push(2); // Action::FunctionCall
    put_str(&mut out, method);
    out.extend_from_slice(&(args.len() as u32).to_le_bytes());
    out.extend_from_slice(args);
    out.extend_from_slice(&gas.to_le_bytes());
    out.extend_from_slice(&0u128.to_le_bytes()); // deposit
    out
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Signs a transaction: its borsh bytes, then the ed25519 signature of their
/// SHA-256.
pub(crate) fn sign(tx: &[u8], key: &SigningKey) -> Vec<u8> {
    let hash = Sha256::digest(tx);
    let sig = key.sign(&hash);
    let mut out = tx.to_vec();
    out.push(0); // KeyType::ED25519
    out.extend_from_slice(&sig.to_bytes());
    out
}

/// Calls `method` on `contract` with `args`, signed with `key`, and waits
/// until the transaction is final. Returns the JSON value that the method
/// returns. A refused call returns its error message as [`Error::Rpc`].
pub(crate) fn call(
    rpc_url: &str,
    key: &KeyFile,
    contract: &str,
    method: &str,
    args: &Value,
) -> Result<Value, Error> {
    if key.account_id.is_empty() {
        return Err(Error::Io(
            "the key file has no account_id: give the account of the key".into(),
        ));
    }
    let signing = key.signing_key()?;
    let public: [u8; 32] = signing.verifying_key().to_bytes();
    let access = rpc::post(
        rpc_url,
        json!({"jsonrpc": "2.0", "id": "builddeed", "method": "query", "params": {
            "request_type": "view_access_key", "finality": "final",
            "account_id": key.account_id, "public_key": key.public_key}}),
    )?;
    let nonce = access["nonce"]
        .as_u64()
        .ok_or_else(|| Error::Rpc("the access key has no nonce".into()))?;
    let hash = bs58::decode(access["block_hash"].as_str().unwrap_or(""))
        .into_vec()
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| Error::Rpc("the reply has no block hash".into()))?;
    let tx = encode_call(
        &key.account_id,
        &public,
        nonce + 1,
        contract,
        &hash,
        method,
        args.to_string().as_bytes(),
        GAS,
    );
    let signed = sign(&tx, &signing);
    let result = rpc::post(
        rpc_url,
        json!({"jsonrpc": "2.0", "id": "builddeed", "method": "send_tx", "params": {
            "signed_tx_base64": B64.encode(&signed), "wait_until": "FINAL"}}),
    )?;
    outcome(&result)
}

/// The return value of a final transaction, or its failure as an error.
fn outcome(result: &Value) -> Result<Value, Error> {
    let status = &result["status"];
    if let Some(v) = status.get("SuccessValue").and_then(Value::as_str) {
        if v.is_empty() {
            return Ok(Value::Null);
        }
        let bytes = B64
            .decode(v)
            .map_err(|e| Error::Rpc(format!("bad return value: {e}")))?;
        return serde_json::from_slice(&bytes)
            .map_err(|e| Error::Rpc(format!("bad return value: {e}")));
    }
    if let Some(f) = status.get("Failure") {
        // The contract's panic message, when there is one.
        let msg = f
            .pointer("/ActionError/kind/FunctionCallError/ExecutionError")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| f.to_string());
        return Err(Error::Rpc(format!("the contract refused the call: {msg}")));
    }
    Err(Error::Rpc(format!(
        "unexpected transaction result: {status}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    // A key made for these tests only; it holds nothing.
    fn test_key() -> (String, SigningKey) {
        let secret = [7u8; 32];
        let k = SigningKey::from_bytes(&secret);
        let mut both = secret.to_vec();
        both.extend_from_slice(k.verifying_key().as_bytes());
        let json = json!({
            "account_id": "agent.testnet",
            "public_key": format!("ed25519:{}", bs58::encode(k.verifying_key().as_bytes()).into_string()),
            "private_key": format!("ed25519:{}", bs58::encode(&both).into_string()),
        });
        (json.to_string(), k)
    }

    #[test]
    fn key_file_parses_and_hides_the_private_key() {
        let (json, k) = test_key();
        let f = KeyFile::parse(&json).unwrap();
        assert_eq!(f.account_id, "agent.testnet");
        assert_eq!(f.signing_key().unwrap().to_bytes(), k.to_bytes());
        assert!(!format!("{f:?}").contains("private"));
    }

    #[test]
    fn key_file_with_another_public_key_is_refused() {
        let (json, _) = test_key();
        let mut v: Value = serde_json::from_str(&json).unwrap();
        v["public_key"] = json!(format!("ed25519:{}", bs58::encode([1u8; 32]).into_string()));
        assert!(KeyFile::parse(&v.to_string()).is_err());
        v["public_key"] = json!("secp256k1:abc");
        assert!(KeyFile::parse(&v.to_string()).is_err());
    }

    #[test]
    fn call_encoding_matches_borsh_layout() {
        let tx = encode_call("a", &[1; 32], 5, "c", &[2; 32], "m", b"{}", 9);
        let mut want = vec![1, 0, 0, 0, b'a', 0];
        want.extend([1u8; 32]);
        want.extend(5u64.to_le_bytes());
        want.extend([1, 0, 0, 0, b'c']);
        want.extend([2u8; 32]);
        want.extend([1, 0, 0, 0, 2]);
        want.extend([1, 0, 0, 0, b'm']);
        want.extend([2, 0, 0, 0, b'{', b'}']);
        want.extend(9u64.to_le_bytes());
        want.extend([0u8; 16]);
        assert_eq!(tx, want);
    }

    #[test]
    fn signature_covers_the_sha256_of_the_transaction() {
        let (_, k) = test_key();
        let tx = encode_call("a", &[1; 32], 5, "c", &[2; 32], "m", b"{}", 9);
        let signed = sign(&tx, &k);
        assert_eq!(&signed[..tx.len()], &tx[..]);
        assert_eq!(signed[tx.len()], 0);
        let sig = ed25519_dalek::Signature::from_slice(&signed[tx.len() + 1..]).unwrap();
        k.verifying_key()
            .verify(&Sha256::digest(&tx), &sig)
            .unwrap();
    }

    #[test]
    fn outcome_returns_the_value_or_the_contract_error() {
        let ok = json!({"status": {"SuccessValue": B64.encode(br#"{"token_id":"doom:1"}"#)}});
        assert_eq!(outcome(&ok).unwrap()["token_id"], "doom:1");
        assert_eq!(
            outcome(&json!({"status": {"SuccessValue": ""}})).unwrap(),
            Value::Null
        );
        let fail = json!({"status": {"Failure": {"ActionError": {"index": 0, "kind": {
            "FunctionCallError": {"ExecutionError": "Smart contract panicked: The agent budget is too low"}}}}}});
        let e = outcome(&fail).unwrap_err();
        assert!(e.to_string().contains("budget is too low"), "{e}");
    }
}
