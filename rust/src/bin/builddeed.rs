//! `builddeed`: buy, register and download BuildDeed products with no
//! browser, for agents and CI jobs. Every command prints one JSON object on
//! standard output and exits with 0 on success, 1 on a failure.
//!
//! ```text
//! builddeed --product checksum budget --key key.json
//! builddeed --product checksum buy --key key.json
//! builddeed --product checksum register --key key.json [--token checksum:1]
//! builddeed --product checksum download [--platform linux-x86_64] [--out DIR]
//! builddeed --product checksum status
//! builddeed --product checksum device-id
//! ```
//!
//! Options: `--contract` (default: the official testnet contract),
//! `--version` (the product version that `status` asks about; default: the
//! last recorded build for this platform), `--rpc` (an https RPC URL),
//! `--account` (the account of the key, when the key file does not name it).

use builddeed::{Client, Error, KeyFile, Product, Status, OFFICIAL_TESTNET_CONTRACT};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: builddeed --product <key> [--contract <account>] [--rpc <url>] <command>
commands:
  budget --key <file>                  the owner's agent budget and licenses of the product
  buy --key <file>                     buy a license from the budget (nothing is sent if a current one is owned)
  register --key <file> [--token <id>] give THIS computer a seat (not a container or sandbox)
  download [--platform <p>] [--out <dir>]  the official file, SHA-256 checked
  status [--version <v>]               the seat of this computer
  device-id                            the computer id of this computer
options: --account <id> names the key's account when the key file does not";

/// Options that take a value. Any other `--` argument is refused.
const OPTIONS: [&str; 9] = [
    "--product",
    "--contract",
    "--rpc",
    "--key",
    "--account",
    "--token",
    "--platform",
    "--out",
    "--version",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The value of `--name`, if given.
fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The command: the one argument that is not an option or an option's value.
/// Refuses unknown options, a missing option value and a second command.
fn command(args: &[String]) -> Result<&str, String> {
    let mut found = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            if !OPTIONS.contains(&a.as_str()) {
                return Err(format!("unknown option {a}"));
            }
            if args.get(i + 1).is_none_or(|v| v.starts_with("--")) {
                return Err(format!("{a} needs a value"));
            }
            i += 2;
        } else {
            if found.is_some() {
                return Err(format!("one command only, not also {a}"));
            }
            found = Some(a.as_str());
            i += 1;
        }
    }
    found.ok_or_else(|| "give a command".to_string())
}

/// The JSON of a failure. A sent transaction with an unknown result names
/// its hash, so that the caller checks it before it tries again.
fn failure(e: &Error) -> Value {
    match e {
        Error::Unconfirmed { tx_hash } => {
            json!({ "error": e.to_string(), "unconfirmed_tx": tx_hash })
        }
        _ => json!({ "error": e.to_string() }),
    }
}

fn run(args: &[String]) -> Result<Value, Value> {
    let usage = |m: String| json!({ "error": m, "usage": USAGE });
    let cmd = command(args).map_err(usage)?;
    let key_name = opt(args, "--product").ok_or_else(|| usage("give --product".into()))?;
    let contract = opt(args, "--contract").unwrap_or_else(|| OFFICIAL_TESTNET_CONTRACT.into());
    let platform = opt(args, "--platform").unwrap_or_else(builddeed::platform);
    let mut product = Product::testnet(&contract, &key_name, "0");
    if let Some(rpc) = opt(args, "--rpc") {
        product = product.with_rpc_url(rpc);
    }
    // What the answers trust: printed with each verdict.
    let source = json!({ "contract": contract, "rpc": product.rpc_url });
    let base = Client::new(product.clone());
    let e = |e: Error| failure(&e);
    let load_key = || -> Result<KeyFile, Value> {
        let path = opt(args, "--key").ok_or_else(|| usage("give --key <key file>".into()))?;
        let k = KeyFile::load(&PathBuf::from(path)).map_err(e)?;
        Ok(match opt(args, "--account") {
            Some(a) => k.with_account(a),
            None => k,
        })
    };
    match cmd {
        "device-id" => Ok(json!({ "device_id": base.device_id().map_err(e)? })),
        "budget" => {
            let key = load_key()?;
            let account = key.account_id().to_string();
            let licenses: Vec<Value> = base
                .licenses_of(&account)
                .map_err(e)?
                .into_iter()
                .map(|l| {
                    json!({ "token_id": l.token_id, "updates_until_ms": l.updates_until_ms,
                            "seats_used": l.devices.len() })
                })
                .collect();
            Ok(json!({
                "account_id": account,
                "budget": base.budget(&account).map_err(e)?,
                "licenses": licenses,
                "source": source,
            }))
        }
        "status" => {
            let version = match opt(args, "--version") {
                Some(v) => v,
                None => base
                    .latest_build(&platform)
                    .map_err(e)?
                    .map(|m| m.version)
                    .unwrap_or_else(|| "0".into()),
            };
            product.version = version.clone();
            let c = Client::new(product);
            let status = match c.status().map_err(e)? {
                Status::Registered(s) => json!({
                    "registered": true, "token_id": s.token_id, "owner_id": s.account_id,
                    "updates_until_ms": s.updates_until_ms }),
                Status::LicenseEnded { token_id } => json!({
                    "registered": false, "token_id": token_id,
                    "reason": "the license ended before this version" }),
                Status::NoSeat => json!({ "registered": false, "reason": "no seat" }),
            };
            Ok(json!({ "product": key_name, "version": version,
                       "device_id": c.device_id().map_err(e)?, "status": status, "source": source }))
        }
        "buy" => {
            let key = load_key()?;
            let p = base.buy_with_key(&key).map_err(e)?;
            Ok(json!({ "token_id": p.token_id, "owner_id": key.account_id(), "bought": p.new }))
        }
        "register" => {
            let key = load_key()?;
            let token = opt(args, "--token");
            let token = base.register_with_key(&key, token.as_deref()).map_err(e)?;
            Ok(json!({ "token_id": token, "device_id": base.device_id().map_err(e)? }))
        }
        "download" => {
            let out = PathBuf::from(opt(args, "--out").unwrap_or_else(|| ".".into()));
            let (path, m) = base.download_latest(&platform, &out).map_err(e)?;
            Ok(
                json!({ "path": path, "file": m.artifact.file, "platform": m.artifact.platform,
                       "version": m.version, "commit": m.commit, "sha256": m.artifact.sha256,
                       "attestation_url": m.artifact.attestation_url, "source": source }),
            )
        }
        other => Err(usage(format!("unknown command {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn command_skips_options_and_their_values() {
        assert_eq!(command(&a("--product doom status")), Ok("status"));
        assert_eq!(
            command(&a("--product doom register --key k.json")),
            Ok("register")
        );
        assert!(command(&a("--product doom")).is_err());
        assert_eq!(
            opt(&a("--product doom buy --key k.json"), "--key").as_deref(),
            Some("k.json")
        );
    }

    #[test]
    fn unknown_options_and_missing_values_are_refused() {
        assert!(
            command(&a("--product doom download --platfrom linux-x86_64"))
                .unwrap_err()
                .contains("--platfrom")
        );
        assert!(command(&a("--product doom buy --key")).is_err());
        assert!(command(&a("--product doom buy register")).is_err());
    }

    #[test]
    fn missing_product_prints_the_usage() {
        let e = run(&a("status")).unwrap_err();
        assert!(e["usage"].as_str().unwrap().starts_with("usage"));
    }
}
