//! `builddeed`: buy, register and download BuildDeed products with no
//! browser, for agents and CI jobs. Every command prints one JSON object on
//! standard output and exits with 0 on success, 1 on a failure.
//!
//! ```text
//! builddeed --product doom status
//! builddeed --product doom buy --key key.json
//! builddeed --product doom register --key key.json [--token doom:1]
//! builddeed --product doom download [--platform linux-x86_64] [--out DIR]
//! builddeed --product doom device-id
//! ```
//!
//! Options: `--contract` (default: the official testnet contract),
//! `--version` (the version that `status` asks about; default: the newest
//! build for this platform), `--rpc` (an https RPC URL), `--account` (the
//! account of the key, when the key file does not name it).

use builddeed::{Client, KeyFile, Product, Status, OFFICIAL_TESTNET_CONTRACT};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: builddeed --product <key> [--contract <account>] [--version <v>] [--rpc <url>] [--account <id>] \
<status | buy --key <file> | register --key <file> [--token <id>] | download [--platform <p>] [--out <dir>] | device-id>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", json!({ "error": e }));
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

/// The first argument that is not an option or an option's value.
fn command(args: &[String]) -> Option<&str> {
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = true;
        } else {
            return Some(a);
        }
    }
    None
}

fn run(args: &[String]) -> Result<Value, String> {
    let key_name = opt(args, "--product").ok_or(USAGE)?;
    let contract = opt(args, "--contract").unwrap_or_else(|| OFFICIAL_TESTNET_CONTRACT.into());
    let platform = opt(args, "--platform").unwrap_or_else(builddeed::platform);
    let mut product = Product::testnet(&contract, &key_name, "0");
    if let Some(rpc) = opt(args, "--rpc") {
        product = product.with_rpc_url(rpc);
    }
    let base = Client::new(product.clone());
    let e = |e: builddeed::Error| e.to_string();
    let load_key = || -> Result<KeyFile, String> {
        let path = opt(args, "--key").ok_or("give --key <key file>")?;
        let k = KeyFile::load(&PathBuf::from(path)).map_err(e)?;
        Ok(match opt(args, "--account") {
            Some(a) => k.with_account(a),
            None => k,
        })
    };
    match command(args).ok_or(USAGE)? {
        "device-id" => Ok(json!({ "device_id": base.device_id().map_err(e)? })),
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
                       "device_id": c.device_id().map_err(e)?, "status": status }))
        }
        "buy" => {
            let key = load_key()?;
            let token = base.buy_with_key(&key).map_err(e)?;
            Ok(json!({ "token_id": token, "owner_id": key.account_id }))
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
                json!({ "path": path, "version": m.version, "commit": m.commit,
                       "sha256": m.artifact.sha256, "attestation_url": m.artifact.attestation_url }),
            )
        }
        other => Err(format!("unknown command {other}. {USAGE}")),
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
        assert_eq!(command(&a("--product doom status")), Some("status"));
        assert_eq!(
            command(&a("--product doom register --key k.json")),
            Some("register")
        );
        assert_eq!(command(&a("--product doom")), None);
        assert_eq!(
            opt(&a("--product doom buy --key k.json"), "--key").as_deref(),
            Some("k.json")
        );
    }

    #[test]
    fn missing_product_prints_the_usage() {
        assert!(run(&a("status")).unwrap_err().starts_with("usage"));
    }
}
