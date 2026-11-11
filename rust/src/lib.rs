//! # BuildDeed SDK
//!
//! Sell a Rust program with [BuildDeed](https://github.com/13rac1/builddeed):
//! a license is an NFT on NEAR with computer seats, and the license contract
//! keeps the record of your official builds.
//!
//! Your program asks one question at start: does this computer have a seat
//! on a license that covers this version? Keep the program useful without a
//! license, and turn on the extra feature when the answer is yes.
//!
//! ```no_run
//! use builddeed::{Client, Product};
//!
//! let client = Client::new(Product::testnet(
//!     "doom.builddeed.testnet", // your license contract
//!     "doom",                   // your product key on the BuildDeed website
//!     env!("CARGO_PKG_VERSION"),
//! ));
//! match client.check() {
//!     Ok(Some(seat)) => println!("Registered to {}", seat.account_id),
//!     Ok(None) => {
//!         println!("Unregistered. Register this computer:");
//!         client.open_register_page().ok();
//!     }
//!     Err(e) => println!("Cannot check the license, running unregistered: {e}"),
//! }
//! ```
//!
//! There is no offline mode: a result stored on the computer could be copied
//! or edited. With no network, run unregistered.
//!
//! The check is in your public source code. Anyone may build your program
//! without it; the official build is the one that your contract records.

use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

mod device;
mod rpc;

#[cfg(feature = "bevy")]
pub mod bevy_plugin;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_rpc;

/// A NEAR network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// NEAR testnet: free test tokens.
    Testnet,
    /// NEAR mainnet.
    Mainnet,
}

/// The product that this program sells: its license contract and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    /// The account of the license contract, for example `doom.builddeed.testnet`.
    pub contract: String,
    /// The product key on the BuildDeed website, for example `doom`.
    pub key: String,
    /// The version of this program, as the release records it.
    pub version: String,
    /// The network of the contract.
    pub network: Network,
    /// The NEAR JSON-RPC endpoint.
    pub rpc_url: String,
    /// The BuildDeed website.
    pub site_url: String,
}

impl Product {
    /// A product on NEAR testnet, with the default RPC endpoint and website.
    pub fn testnet(contract: &str, key: &str, version: &str) -> Self {
        Self {
            contract: contract.into(),
            key: key.into(),
            version: version.into(),
            network: Network::Testnet,
            rpc_url: "https://rpc.testnet.fastnear.com".into(),
            site_url: "https://13rac1.github.io/builddeed".into(),
        }
    }

    /// A product on NEAR mainnet, with the default RPC endpoint and website.
    pub fn mainnet(contract: &str, key: &str, version: &str) -> Self {
        Self {
            network: Network::Mainnet,
            rpc_url: "https://rpc.mainnet.fastnear.com".into(),
            ..Self::testnet(contract, key, version)
        }
    }

    /// The same product with another RPC endpoint.
    pub fn with_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.rpc_url = url.into();
        self
    }

    /// The same product with another website.
    pub fn with_site_url(mut self, url: impl Into<String>) -> Self {
        self.site_url = url.into();
        self
    }
}

/// The seat of this computer on a license that covers this version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seat {
    /// The owner of the license, for example `alice.testnet`.
    pub account_id: String,
    /// The license (NFT) token id.
    pub token_id: String,
    /// The license covers builds released at or before this time (Unix ms).
    pub updates_until_ms: u64,
}

/// What the contract says about this computer, for this version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// A seat on a license that covers this version: registered.
    Registered(Seat),
    /// A seat, but the license ended before this version: tell the user to
    /// renew.
    LicenseEnded {
        /// The license token id.
        token_id: String,
    },
    /// No seat: unregistered.
    NoSeat,
}

/// The computer id for a contract and a machine id: lowercase hex of
/// SHA-256("builddeed-device-v1\n" + contract + "\n" + machine_id). See
/// `spec/README.md`. [`Client::new`] reads the machine id of this computer.
pub fn device_id_from(contract: &str, machine_id: &str) -> String {
    device::id_from(contract, machine_id)
}

/// One file of an official build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    /// For example `linux-x86_64`.
    pub platform: String,
    /// The file name of the release.
    pub file: String,
    /// SHA-256 of the file, lowercase hex.
    pub sha256: String,
    /// The URL of the provenance attestation.
    pub attestation_url: Option<String>,
}

/// The official build that contains a file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ArtifactMatch {
    /// The version of the build.
    pub version: String,
    /// The git commit of the build.
    pub commit: String,
    /// When the build was first recorded (Unix ms).
    pub released_at_ms: u64,
    /// The file.
    pub artifact: Artifact,
}

/// A failure of the SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The operating system gives no machine id, so the computer has no id.
    NoMachineId,
    /// The RPC endpoint cannot be reached.
    Network(String),
    /// The RPC endpoint answered with an error or an unexpected reply.
    Rpc(String),
    /// The computer has a seat, but its license ended before this version.
    LicenseEnded {
        /// The license token id.
        token_id: String,
    },
    /// `wait_for_seat` was stopped.
    Stopped,
    /// `wait_for_seat` waited for the whole time.
    TimedOut,
    /// A local file or program failed (for example the browser).
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NoMachineId => f.write_str("cannot read the machine id of this computer"),
            Error::Network(e) => write!(f, "cannot reach the NEAR RPC endpoint: {e}"),
            Error::Rpc(e) => write!(f, "NEAR RPC error: {e}"),
            Error::LicenseEnded { token_id } => write!(
                f,
                "license #{token_id} ended before this version; renew it on the website"
            ),
            Error::Stopped => f.write_str("stopped waiting for the registration"),
            Error::TimedOut => f.write_str("the registration did not arrive in time"),
            Error::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

/// The BuildDeed client of one product on this computer.
#[derive(Debug, Clone)]
pub struct Client {
    product: Product,
    device: Result<String, Error>,
}

impl Client {
    /// A client for `product`, with this computer's id.
    pub fn new(product: Product) -> Self {
        let device = device::device_id(&product.contract).map_err(|_| Error::NoMachineId);
        Self { product, device }
    }

    /// A client with a given computer id, for tests.
    pub fn with_device(product: Product, device_id: impl Into<String>) -> Self {
        Self {
            product,
            device: Ok(device_id.into()),
        }
    }

    /// The product of this client.
    pub fn product(&self) -> &Product {
        &self.product
    }

    /// This computer's id: the SHA-256 of the operating system's machine id
    /// and the contract name, 64 lowercase hex characters. The id is computed
    /// at each start and never stored, so a copied file does not move a seat.
    pub fn device_id(&self) -> Result<&str, Error> {
        self.device.as_deref().map_err(Clone::clone)
    }

    /// The first and last characters of the computer id, as the website
    /// shows them.
    pub fn short_device_id(&self) -> Result<String, Error> {
        let d = self.device_id()?;
        Ok(format!("{}...{}", &d[..8], &d[d.len() - 4..]))
    }

    /// The seat of this computer on a license that covers this version, or
    /// `None`. One view call; call it at each start.
    pub fn check(&self) -> Result<Option<Seat>, Error> {
        match self.status()? {
            Status::Registered(s) => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    /// Like [`Client::check`], but tells "license ended" apart from "no seat".
    pub fn status(&self) -> Result<Status, Error> {
        Ok(match self.raw_seat()? {
            Some(d) if d.covers_version => Status::Registered(seat(d)),
            Some(d) => Status::LicenseEnded {
                token_id: d.token_id,
            },
            None => Status::NoSeat,
        })
    }

    fn raw_seat(&self) -> Result<Option<rpc::DeviceLicense>, Error> {
        let device = self.device_id()?;
        rpc::device_license(
            &self.product.rpc_url,
            &self.product.contract,
            device,
            &self.product.version,
        )
    }

    /// The website page that registers this computer.
    pub fn register_url(&self) -> Result<String, Error> {
        let p = &self.product;
        Ok(format!(
            "{}/register.html#product={}&device={}&version={}",
            p.site_url,
            p.key,
            self.device_id()?,
            p.version
        ))
    }

    /// The website page that sells a license.
    pub fn buy_url(&self) -> String {
        format!(
            "{}/buy.html?product={}",
            self.product.site_url, self.product.key
        )
    }

    /// Opens the register page in the default browser.
    pub fn open_register_page(&self) -> Result<(), Error> {
        open_url(&self.register_url()?)
    }

    /// Opens the buy page in the default browser.
    pub fn open_buy_page(&self) -> Result<(), Error> {
        open_url(&self.buy_url())
    }

    /// Asks the contract for this computer's seat every `every`, until it has
    /// a seat, `stop` is set, or `max` has passed. Network errors are tried
    /// again. Blocks: call it from a worker thread after `open_register_page`.
    pub fn wait_for_seat(
        &self,
        stop: &AtomicBool,
        every: Duration,
        max: Duration,
    ) -> Result<Seat, Error> {
        let deadline = Instant::now() + max;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Err(Error::Stopped);
            }
            if Instant::now() >= deadline {
                return Err(Error::TimedOut);
            }
            match self.raw_seat() {
                Ok(Some(d)) if d.covers_version => return Ok(seat(d)),
                Ok(Some(d)) => {
                    return Err(Error::LicenseEnded {
                        token_id: d.token_id,
                    })
                }
                Ok(None) | Err(Error::Network(_)) | Err(Error::Rpc(_)) => std::thread::sleep(every),
                Err(e) => return Err(e),
            }
        }
    }

    /// The official build that contains a file with this SHA-256.
    pub fn find_artifact(&self, sha256: &str) -> Result<Option<ArtifactMatch>, Error> {
        rpc::find_artifact(&self.product.rpc_url, &self.product.contract, sha256)
    }

    /// The official build record of the running program, or `None` if the
    /// contract has no record of it (for example a self-built copy).
    pub fn verify_self(&self) -> Result<Option<ArtifactMatch>, Error> {
        let exe = std::env::current_exe().map_err(|e| Error::Io(e.to_string()))?;
        let sha = sha256_file(&exe).map_err(|e| Error::Io(e.to_string()))?;
        self.find_artifact(&sha)
    }
}

fn seat(d: rpc::DeviceLicense) -> Seat {
    Seat {
        account_id: d.owner_id,
        token_id: d.token_id,
        updates_until_ms: d.updates_until_ms,
    }
}

/// SHA-256 of a file, lowercase hex, read in 64 KiB blocks.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn open_url(url: &str) -> Result<(), Error> {
    let result = if cfg!(target_os = "windows") {
        // Not "cmd /C start": cmd reads each "&" in the URL as a command separator.
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| Error::Io(format!("cannot open {url}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64-hex computer id: 6844b8b0, 52 zeros, af75.
    fn dev() -> String {
        format!("6844b8b0{}af75", "0".repeat(52))
    }

    fn client(url: String) -> Client {
        let product = Product::testnet("doom.builddeed.testnet", "doom", "0.2.0").with_rpc_url(url);
        Client::with_device(product, dev())
    }

    #[test]
    fn urls_name_product_device_and_version() {
        let c = client("http://x".into());
        assert_eq!(
            c.register_url().unwrap(),
            format!(
                "https://13rac1.github.io/builddeed/register.html#product=doom&device={}&version=0.2.0",
                dev()
            )
        );
        assert_eq!(
            c.buy_url(),
            "https://13rac1.github.io/builddeed/buy.html?product=doom"
        );
        assert_eq!(dev().len(), 64);
        assert_eq!(c.short_device_id().unwrap(), "6844b8b0...af75");
    }

    #[test]
    fn check_needs_a_seat_that_covers_the_version() {
        let url = fake_rpc::serve(vec![
            fake_rpc::seat("4", "bob.testnet", true),
            fake_rpc::seat("4", "bob.testnet", false),
            fake_rpc::no_seat(),
        ]);
        let c = client(url);
        let s = c.check().unwrap().expect("seat");
        assert_eq!(
            (s.account_id.as_str(), s.token_id.as_str()),
            ("bob.testnet", "4")
        );
        assert_eq!(c.check(), Ok(None), "license ended before this version");
        assert_eq!(c.check(), Ok(None), "no seat");
    }

    #[test]
    fn wait_for_seat_retries_then_returns_the_seat() {
        let url = fake_rpc::serve(vec![
            fake_rpc::no_seat(),
            fake_rpc::error("INTERNAL_ERROR"),
            fake_rpc::seat("1", "a.testnet", true),
        ]);
        let s = client(url)
            .wait_for_seat(
                &AtomicBool::new(false),
                Duration::from_millis(20),
                Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(s.token_id, "1");
    }

    #[test]
    fn wait_for_seat_reports_an_ended_license_a_stop_and_a_timeout() {
        let url = fake_rpc::serve(vec![fake_rpc::seat("2", "a.testnet", false)]);
        let c = client(url);
        let quick = Duration::from_millis(20);
        assert_eq!(
            c.wait_for_seat(&AtomicBool::new(false), quick, Duration::from_secs(10)),
            Err(Error::LicenseEnded {
                token_id: "2".into()
            })
        );
        assert_eq!(
            c.wait_for_seat(&AtomicBool::new(true), quick, Duration::from_secs(10)),
            Err(Error::Stopped)
        );
        assert_eq!(
            c.wait_for_seat(&AtomicBool::new(false), quick, Duration::ZERO),
            Err(Error::TimedOut)
        );
    }

    #[test]
    fn find_artifact_decodes_the_record() {
        let record = serde_json::json!({
            "version": "0.2.0", "commit": "a".repeat(40), "released_at_ms": 5u64,
            "artifact": { "platform": "linux-x86_64", "file": "f", "sha256": "b".repeat(64),
                          "attestation_url": null }
        });
        let url = fake_rpc::serve(vec![
            fake_rpc::view(record),
            fake_rpc::view(serde_json::Value::Null),
        ]);
        let c = client(url);
        assert_eq!(
            c.find_artifact(&"b".repeat(64)).unwrap().unwrap().version,
            "0.2.0"
        );
        assert_eq!(c.find_artifact(&"c".repeat(64)), Ok(None));
    }

    #[test]
    fn no_machine_id_is_an_error_for_device_calls() {
        let c = Client {
            product: Product::testnet("c", "k", "1"),
            device: Err(Error::NoMachineId),
        };
        assert_eq!(c.check(), Err(Error::NoMachineId));
        assert_eq!(c.register_url(), Err(Error::NoMachineId));
        assert!(c.buy_url().ends_with("product=k"));
    }

    #[test]
    fn sha256_file_matches_a_known_value() {
        let p = std::env::temp_dir().join(format!("bd-sdk-{}", std::process::id()));
        std::fs::write(&p, "one").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "7692c3ad3540bb803c020b3aee66cd8887123234ea0c6e7143c0add73ff431ed"
        );
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn mainnet_uses_the_mainnet_endpoint() {
        let p = Product::mainnet("x.builddeed.near", "x", "1.0.0");
        assert_eq!(p.network, Network::Mainnet);
        assert!(p.rpc_url.contains("mainnet"));
    }
}
