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
/// name. The private key is checked against the public key when the file is
/// read, and it is never printed.
#[derive(Clone)]
pub struct KeyFile {
    account_id: String,
    signing: SigningKey,
}

/// The JSON of a key file, before its check.
#[derive(Deserialize)]
struct RawKeyFile {
    #[serde(default)]
    account_id: String,
    public_key: String,
    private_key: String,
}

impl std::fmt::Debug for KeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the private key.
        f.debug_struct("KeyFile")
            .field("account_id", &self.account_id)
            .field("public_key", &self.public_key())
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
                .filter(|n| is_account_id(n))
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

    /// The account of the key; empty when the file does not name it.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// `ed25519:` and the base58 public key.
    pub fn public_key(&self) -> String {
        format!(
            "ed25519:{}",
            bs58::encode(self.signing.verifying_key().as_bytes()).into_string()
        )
    }

    /// Parses the JSON of a key file and checks that the private key matches
    /// the public key.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let raw: RawKeyFile = serde_json::from_str(json)
            .map_err(|e| Error::Io(format!("not a NEAR key file: {e}")))?;
        // near-cli writes the 64-byte form: the secret, then the public key.
        let private = decode_key(&raw.private_key, 64)?;
        let secret: [u8; 32] = private[..32].try_into().expect("64 bytes checked");
        let signing = SigningKey::from_bytes(&secret);
        let public = decode_key(&raw.public_key, 32)?;
        if signing.verifying_key().as_bytes()[..] != public[..] {
            return Err(Error::Io(
                "the private key of the key file does not match its public key".into(),
            ));
        }
        Ok(KeyFile {
            account_id: raw.account_id,
            signing,
        })
    }
}

/// A NEAR account id as a folder name: a named account (with a dot, lowercase
/// letters, digits, `-`, `_`) or a 64-hex implicit account.
fn is_account_id(s: &str) -> bool {
    (2..=64).contains(&s.len())
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
        && (s.contains('.') || (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())))
        && !s.starts_with('.')
        && !s.ends_with('.')
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

/// The fixed parts of a transaction: who signs, with which key, for which
/// contract.
pub(crate) struct Header<'a> {
    pub signer: &'a str,
    pub public_key: [u8; 32],
    pub receiver: &'a str,
}

/// The borsh bytes of a `Transaction` with one `FunctionCall` action, [`GAS`]
/// and no deposit.
pub(crate) fn encode_call(
    h: &Header,
    nonce: u64,
    block_hash: &[u8; 32],
    method: &str,
    args: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    put_bytes(&mut out, h.signer.as_bytes())?;
    out.push(0); // KeyType::ED25519
    out.extend_from_slice(&h.public_key);
    out.extend_from_slice(&nonce.to_le_bytes());
    put_bytes(&mut out, h.receiver.as_bytes())?;
    out.extend_from_slice(block_hash);
    out.extend_from_slice(&1u32.to_le_bytes()); // one action
    out.push(2); // Action::FunctionCall
    put_bytes(&mut out, method.as_bytes())?;
    put_bytes(&mut out, args)?;
    out.extend_from_slice(&GAS.to_le_bytes());
    out.extend_from_slice(&0u128.to_le_bytes()); // deposit
    Ok(out)
}

/// A borsh string or byte vector: u32 length, then the bytes.
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) -> Result<(), Error> {
    let len = u32::try_from(b.len()).map_err(|_| Error::Io("an argument is too long".into()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(b);
    Ok(())
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

/// The methods that an agent key may call. A key that allows any other
/// method, or all methods, is refused: an agent must not hold it.
pub(crate) const AGENT_METHODS: [&str; 2] = ["buy_from_budget", "add_device_from_budget"];

/// Refuses an access key that is not an agent key for `contract`: a
/// full-access key, a key for another contract, or a key that allows other
/// methods.
fn check_agent_key(permission: &Value, contract: &str) -> Result<(), Error> {
    let fc = permission.get("FunctionCall").ok_or_else(|| {
        Error::Io(
            "this is a full-access key: give the agent a function-call key for buy_from_budget and add_device_from_budget".into(),
        )
    })?;
    if fc["receiver_id"].as_str() != Some(contract) {
        return Err(Error::Io(format!(
            "this key is for {}, not for {contract}",
            fc["receiver_id"]
        )));
    }
    let methods: Vec<&str> = fc["method_names"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if methods.is_empty() || methods.iter().any(|m| !AGENT_METHODS.contains(m)) {
        return Err(Error::Io(
            "this key allows other methods: give the agent a key for buy_from_budget and add_device_from_budget only".into(),
        ));
    }
    Ok(())
}

/// Calls `method` on `contract` with `args`, signed with `key`, and waits
/// until the transaction is final. Returns the JSON value that the method
/// returns. A refused call returns its error message as [`Error::Rpc`]. When
/// the transaction was sent but its result is unknown (a time-out or a lost
/// connection), returns [`Error::Unconfirmed`] with its hash: do not send it
/// again before you check the result.
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
    let access = rpc::post(
        rpc_url,
        json!({"jsonrpc": "2.0", "id": "builddeed", "method": "query", "params": {
            "request_type": "view_access_key", "finality": "final",
            "account_id": key.account_id, "public_key": key.public_key()}}),
    )?;
    check_agent_key(&access["permission"], contract)?;
    let nonce = access["nonce"]
        .as_u64()
        .ok_or_else(|| Error::Rpc("the access key has no nonce".into()))?;
    let block_hash = bs58::decode(access["block_hash"].as_str().unwrap_or(""))
        .into_vec()
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| Error::Rpc("the reply has no block hash".into()))?;
    let header = Header {
        signer: &key.account_id,
        public_key: key.signing.verifying_key().to_bytes(),
        receiver: contract,
    };
    let tx = encode_call(
        &header,
        nonce + 1,
        &block_hash,
        method,
        args.to_string().as_bytes(),
    )?;
    let tx_hash = bs58::encode(Sha256::digest(&tx)).into_string();
    let signed = sign(&tx, &key.signing);
    let sent = rpc::post(
        rpc_url,
        json!({"jsonrpc": "2.0", "id": "builddeed", "method": "send_tx", "params": {
            "signed_tx_base64": B64.encode(&signed), "wait_until": "FINAL"}}),
    );
    match sent {
        Ok(result) => outcome(&result),
        Err(Error::Network(_)) => Err(Error::Unconfirmed { tx_hash }),
        Err(Error::Rpc(m)) if m.contains("TIMEOUT_ERROR") => Err(Error::Unconfirmed { tx_hash }),
        Err(e) => Err(e),
    }
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
        assert_eq!(f.account_id(), "agent.testnet");
        assert_eq!(f.signing.to_bytes(), k.to_bytes());
        assert!(f.public_key().starts_with("ed25519:"));
        assert!(!format!("{f:?}").contains("signing"));
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
    fn account_folder_names() {
        assert!(is_account_id("alice.testnet"));
        assert!(is_account_id(&"a".repeat(64)));
        assert!(!is_account_id("keys.d."));
        assert!(!is_account_id("agent"));
        assert!(!is_account_id("Alice.testnet"));
    }

    fn header() -> Header<'static> {
        Header {
            signer: "a",
            public_key: [1; 32],
            receiver: "c",
        }
    }

    #[test]
    fn call_encoding_matches_borsh_layout() {
        let tx = encode_call(&header(), 5, &[2; 32], "m", b"{}").unwrap();
        let mut want = vec![1, 0, 0, 0, b'a', 0];
        want.extend([1u8; 32]);
        want.extend(5u64.to_le_bytes());
        want.extend([1, 0, 0, 0, b'c']);
        want.extend([2u8; 32]);
        want.extend([1, 0, 0, 0, 2]);
        want.extend([1, 0, 0, 0, b'm']);
        want.extend([2, 0, 0, 0, b'{', b'}']);
        want.extend(GAS.to_le_bytes());
        want.extend([0u8; 16]);
        assert_eq!(tx, want);
    }

    /// The same bytes as near-cli-rs, and a signature that verifies the same
    /// way: ../fixtures/signed-tx.json.
    #[test]
    fn encoding_matches_near_cli() {
        let f: Value = serde_json::from_str(include_str!("../../fixtures/signed-tx.json")).unwrap();
        let signed = B64.decode(f["signed_tx_base64"].as_str().unwrap()).unwrap();
        let signer = f["signer"].as_str().unwrap();
        let receiver = f["receiver"].as_str().unwrap();
        assert_eq!(f["gas"].as_u64().unwrap(), GAS);
        let public: [u8; 32] = decode_key(f["public_key"].as_str().unwrap(), 32)
            .unwrap()
            .try_into()
            .unwrap();
        // Read the nonce and the block hash at their places in the bytes.
        let at_nonce = 4 + signer.len() + 1 + 32;
        let nonce = u64::from_le_bytes(signed[at_nonce..at_nonce + 8].try_into().unwrap());
        let at_hash = at_nonce + 8 + 4 + receiver.len();
        let block_hash: [u8; 32] = signed[at_hash..at_hash + 32].try_into().unwrap();
        let h = Header {
            signer,
            public_key: public,
            receiver,
        };
        let tx = encode_call(
            &h,
            nonce,
            &block_hash,
            f["method"].as_str().unwrap(),
            f["args"].as_str().unwrap().as_bytes(),
        )
        .unwrap();
        assert_eq!(&signed[..tx.len()], &tx[..]);
        assert_eq!(signed.len(), tx.len() + 1 + 64);
        let sig = ed25519_dalek::Signature::from_slice(&signed[tx.len() + 1..]).unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&public)
            .unwrap()
            .verify(&Sha256::digest(&tx), &sig)
            .unwrap();
        // The SDK writes the same JSON for these arguments.
        assert_eq!(
            json!({"product": "checksum", "price": "100000000000000000000000"}).to_string(),
            f["args"].as_str().unwrap()
        );
    }

    #[test]
    fn signature_covers_the_sha256_of_the_transaction() {
        let (_, k) = test_key();
        let tx = encode_call(&header(), 5, &[2; 32], "m", b"{}").unwrap();
        let signed = sign(&tx, &k);
        assert_eq!(&signed[..tx.len()], &tx[..]);
        assert_eq!(signed[tx.len()], 0);
        let sig = ed25519_dalek::Signature::from_slice(&signed[tx.len() + 1..]).unwrap();
        k.verifying_key()
            .verify(&Sha256::digest(&tx), &sig)
            .unwrap();
    }

    #[test]
    fn only_an_agent_key_for_the_contract_is_used() {
        let c = "licenses.builddeed.testnet";
        let fc = |r: &str, m: Value| json!({"FunctionCall": {"allowance": "1", "receiver_id": r, "method_names": m}});
        assert!(check_agent_key(&json!("FullAccess"), c).is_err());
        assert!(check_agent_key(&fc("other.testnet", json!(["buy_from_budget"])), c).is_err());
        assert!(check_agent_key(&fc(c, json!([])), c).is_err());
        assert!(check_agent_key(&fc(c, json!(["buy_from_budget", "set_prices"])), c).is_err());
        assert!(check_agent_key(
            &fc(c, json!(["buy_from_budget", "add_device_from_budget"])),
            c
        )
        .is_ok());
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
