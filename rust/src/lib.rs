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
//!     "licenses.builddeed.testnet", // the official BuildDeed contract
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
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod device;
mod presence;
mod rpc;
mod tx;

pub use tx::KeyFile;

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

/// The contract of the official BuildDeed collection on NEAR testnet. Every
/// product in it is one NFT collection.
pub const OFFICIAL_TESTNET_CONTRACT: &str = "licenses.builddeed.testnet";

/// The product that this program sells: its license contract and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    /// The account of the license contract, for example
    /// [`OFFICIAL_TESTNET_CONTRACT`].
    pub contract: String,
    /// The product key in the contract (`add_product`), for example `doom`.
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
            site_url: "https://testnet.builddeed.dev".into(),
        }
    }

    /// A product on NEAR mainnet, with the default RPC endpoint and website.
    pub fn mainnet(contract: &str, key: &str, version: &str) -> Self {
        Self {
            network: Network::Mainnet,
            rpc_url: "https://rpc.mainnet.fastnear.com".into(),
            site_url: "https://builddeed.dev".into(),
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

/// The computer id for a contract, a product key and a machine id:
/// lowercase hex of SHA-256("builddeed-device-v1\n" + contract + "\n" +
/// product + "\n" + machine_id). See `spec/README.md`. [`Client::new`] reads
/// the machine id of this computer.
pub fn device_id_from(contract: &str, product: &str, machine_id: &str) -> String {
    device::id_from(contract, product, machine_id)
}

/// The 6-digit confirmation code of a computer id. The program shows it while
/// the register page is open; the page asks for it when the browser blocks
/// the presence check. Decimal of the first 4 bytes (big-endian) of
/// SHA-256("builddeed-confirm-v1\n" + device_id), modulo 1 000 000, with
/// leading zeros. See `spec/README.md`.
pub fn confirm_code_from(device_id: &str) -> String {
    let hash = Sha256::digest(format!("builddeed-confirm-v1\n{device_id}").as_bytes());
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]);
    format!("{:06}", n % 1_000_000)
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
    /// The https URL to download the file, if the release recorded one.
    #[serde(default)]
    pub url: Option<String>,
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
    /// A transaction was sent, but its result is unknown (a time-out or a
    /// lost connection). It can still succeed: check the result (for
    /// example `licenses_for_owner`) before you send it again.
    Unconfirmed {
        /// The transaction hash, base58.
        tx_hash: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NoMachineId => f.write_str(
                "cannot read the machine id of this computer. Run this on the computer that will run the program: a container or a cloud sandbox often has no machine id",
            ),
            Error::Network(e) => write!(f, "cannot reach the NEAR RPC endpoint: {e}"),
            Error::Rpc(e) => write!(f, "NEAR RPC error: {e}"),
            Error::LicenseEnded { token_id } => write!(
                f,
                "license #{token_id} ended before this version; renew it on the website"
            ),
            Error::Stopped => f.write_str("stopped waiting for the registration"),
            Error::TimedOut => f.write_str("the registration did not arrive in time"),
            Error::Io(e) => f.write_str(e),
            Error::Unconfirmed { tx_hash } => write!(
                f,
                "transaction {tx_hash} was sent, but its result is unknown. Check the result before you try again: it can still succeed"
            ),
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
        let device =
            device::device_id(&product.contract, &product.key).map_err(|_| Error::NoMachineId);
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
        Ok(match (d.get(..8), d.get(d.len().saturating_sub(4)..)) {
            (Some(a), Some(b)) if d.len() > 12 => format!("{a}...{b}"),
            _ => d.to_string(),
        })
    }

    /// The 6-digit code that the program shows while the register page is
    /// open (see [`confirm_code_from`]). Show it next to the instruction
    /// "If the page asks for a code, enter this one".
    pub fn confirm_code(&self) -> Result<String, Error> {
        Ok(confirm_code_from(self.device_id()?))
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
            &self.product.key,
            device,
            &self.product.version,
        )
    }

    /// The website page that registers this computer.
    ///
    /// [`Client::open_register_page`] adds the presence check (`port` and
    /// `code`) to this URL.
    pub fn register_url(&self) -> Result<String, Error> {
        let p = &self.product;
        Ok(format!(
            "{}/register/#product={}&device={}&version={}",
            p.site_url,
            encode(&p.key),
            encode(self.device_id()?),
            encode(&p.version)
        ))
    }

    /// The website page that sells a license.
    pub fn buy_url(&self) -> String {
        format!("{}/p/{}/", self.product.site_url, encode(&self.product.key))
    }

    /// Opens the register page in the default browser.
    ///
    /// The presence check: for [`REGISTER_WAIT`], the program answers a
    /// one-time code on 127.0.0.1, and the page asks the owner's wallet only
    /// when that answer names this computer. So a register link sent from
    /// another computer cannot give a seat to that computer. See
    /// `spec/README.md`.
    pub fn open_register_page(&self) -> Result<(), Error> {
        let device = self.device_id()?;
        let presence = presence::start(
            device,
            &self.product.contract,
            &presence::origin_of(&self.product.site_url),
            REGISTER_WAIT,
            Arc::new(AtomicBool::new(false)),
        )?;
        let url = format!(
            "{}&port={}&code={}",
            self.register_url()?,
            presence.port,
            presence.code
        );
        open_url(&url)
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
    ///
    /// The SHA-256 must have 64 hex characters, and the answer must name the
    /// same SHA-256.
    pub fn find_artifact(&self, sha256: &str) -> Result<Option<ArtifactMatch>, Error> {
        let sha256 = sha256.to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Io("a SHA-256 has 64 hex characters".into()));
        }
        let found = rpc::find_artifact(
            &self.product.rpc_url,
            &self.product.contract,
            &self.product.key,
            &sha256,
        )?;
        if let Some(m) = &found {
            if m.artifact.sha256.to_ascii_lowercase() != sha256 {
                return Err(Error::Rpc(
                    "the record is for another file than the one asked for".into(),
                ));
            }
        }
        Ok(found)
    }

    /// The official build record of the running program, or `None` if the
    /// contract has no record of it (for example a self-built copy, or a
    /// program that a release ships inside an archive or app bundle whose
    /// record is the archive).
    ///
    /// This is not a security check: a changed program controls its own
    /// answer. To check a download, hash the file from outside the program
    /// (the BuildDeed website's verify page, or [`Client::find_artifact`] in
    /// another program).
    pub fn verify_self(&self) -> Result<Option<ArtifactMatch>, Error> {
        let exe = std::env::current_exe().map_err(|e| Error::Io(e.to_string()))?;
        let sha = sha256_file(&exe).map_err(|e| Error::Io(e.to_string()))?;
        self.find_artifact(&sha)
    }

    // ----- Agents: no browser -----

    /// The file for `platform` (see [`platform`]) in the last recorded
    /// official build, or `None` when that build has none. The release records its main download for
    /// each platform first, and this is that file.
    pub fn latest_build(&self, platform: &str) -> Result<Option<ArtifactMatch>, Error> {
        let found: Option<ArtifactMatch> = rpc::view(
            &self.product.rpc_url,
            &self.product.contract,
            "latest_build",
            serde_json::json!({ "product": self.product.key, "platform": platform }),
        )?;
        if let Some(m) = &found {
            if m.artifact.platform != platform {
                return Err(Error::Rpc(
                    "the record is for another platform than the one asked for".into(),
                ));
            }
        }
        Ok(found)
    }

    /// Downloads the last recorded official file for `platform` into `dir`,
    /// and checks that its SHA-256 is the recorded one. Returns the path and
    /// the record. The file is written to a temporary name and gets its own
    /// name only after the check, so a failed or changed download never
    /// replaces a file. The check trusts the RPC endpoint and the contract of
    /// this client.
    pub fn download_latest(
        &self,
        platform: &str,
        dir: &std::path::Path,
    ) -> Result<(std::path::PathBuf, ArtifactMatch), Error> {
        let m = self
            .latest_build(platform)?
            .ok_or_else(|| Error::Io(format!("no official build for {platform}")))?;
        let sha = &m.artifact.sha256;
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Rpc("the record has no valid SHA-256".into()));
        }
        let url = m
            .artifact
            .url
            .clone()
            .ok_or_else(|| Error::Io("the record has no download URL".into()))?;
        if !url.starts_with("https://") {
            return Err(Error::Io(format!("the download URL is not https: {url}")));
        }
        let name = std::path::Path::new(&m.artifact.file)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::Io("the record has no file name".into()))?
            .to_string();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .redirects(5)
            .build();
        let resp = agent
            .get(&url)
            .call()
            .map_err(|e| Error::Network(e.to_string()))?;
        if !resp.get_url().starts_with("https://") {
            return Err(Error::Io(
                "the download was redirected away from https".into(),
            ));
        }
        let path = save_checked(
            resp.into_reader(),
            dir,
            &name,
            &m.artifact.sha256,
            MAX_DOWNLOAD,
        )?;
        Ok((path, m))
    }

    /// The agent budget of `account_id`, or `None`.
    pub fn budget(&self, account_id: &str) -> Result<Option<AgentBudget>, Error> {
        rpc::view(
            &self.product.rpc_url,
            &self.product.contract,
            "get_budget",
            serde_json::json!({ "account_id": account_id }),
        )
    }

    /// The licenses of this product that `account_id` owns.
    pub fn licenses_of(&self, account_id: &str) -> Result<Vec<OwnedLicense>, Error> {
        rpc::view(
            &self.product.rpc_url,
            &self.product.contract,
            "licenses_for_owner",
            serde_json::json!({ "account_id": account_id, "product": self.product.key }),
        )
    }

    /// Buys a license of this product for the key's account, paid from the
    /// account's agent budget (`deposit_budget`). `key` must be a
    /// function-call key for `buy_from_budget` and `add_device_from_budget`
    /// only. When the account already owns a license of this product whose
    /// update period has not ended, nothing is sent and that license comes
    /// back with `new: false`, so running this twice buys one license.
    pub fn buy_with_key(&self, key: &KeyFile) -> Result<Purchase, Error> {
        let account = key_account(key)?;
        let now = now_ms();
        if let Some(l) = self
            .licenses_of(account)?
            .into_iter()
            .find(|l| l.updates_until_ms > now)
        {
            return Ok(Purchase {
                token_id: l.token_id,
                new: false,
            });
        }
        #[derive(Deserialize)]
        struct P {
            price: String,
        }
        let p: Option<P> = rpc::view(
            &self.product.rpc_url,
            &self.product.contract,
            "get_product",
            serde_json::json!({ "key": self.product.key }),
        )?;
        let price = p
            .ok_or_else(|| Error::Rpc("the product does not exist".into()))?
            .price;
        let v = tx::call(
            &self.product.rpc_url,
            key,
            &self.product.contract,
            "buy_from_budget",
            &serde_json::json!({ "product": self.product.key, "price": price }),
        )?;
        let token_id = v["token_id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Error::Rpc("the reply has no token id".into()))?;
        Ok(Purchase {
            token_id,
            new: true,
        })
    }

    /// Registers this computer on a license of the key's account, with no
    /// browser, and pays the storage from the account's agent budget.
    /// `token_id` picks the license; with `None`, a license of this product
    /// with a free seat whose update period has not ended. If this computer
    /// already has a seat on that license (or, with `None`, on any license
    /// of the account), returns it and sends nothing.
    ///
    /// The seat is for the computer that runs this: the computer id comes
    /// from its machine id. Run it on the computer that will run the
    /// program, not in a container or a cloud sandbox.
    pub fn register_with_key(
        &self,
        key: &KeyFile,
        token_id: Option<&str>,
    ) -> Result<String, Error> {
        let account = key_account(key)?;
        let device = self.device_id()?.to_string();
        let licenses = self.licenses_of(account)?;
        if let Some(l) = licenses.iter().find(|l| l.devices.contains(&device)) {
            return match token_id {
                Some(t) if t != l.token_id => Err(Error::Io(format!(
                    "this computer has a seat on {} already; remove it there first",
                    l.token_id
                ))),
                _ => Ok(l.token_id.clone()),
            };
        }
        let token = match token_id {
            Some(t) => t.to_string(),
            None => {
                #[derive(Deserialize)]
                struct P {
                    max_devices: usize,
                }
                let p: Option<P> = rpc::view(
                    &self.product.rpc_url,
                    &self.product.contract,
                    "get_product",
                    serde_json::json!({ "key": self.product.key }),
                )?;
                let max = p
                    .ok_or_else(|| Error::Rpc("the product does not exist".into()))?
                    .max_devices;
                let now = now_ms();
                licenses
                    .iter()
                    .filter(|l| l.devices.len() < max && l.updates_until_ms > now)
                    .max_by_key(|l| l.updates_until_ms)
                    .map(|l| l.token_id.clone())
                    .ok_or_else(|| {
                        Error::Io(format!(
                            "{account} has no license of {} with a free seat; buy one or ask the owner to remove a computer",
                            self.product.key
                        ))
                    })?
            }
        };
        tx::call(
            &self.product.rpc_url,
            key,
            &self.product.contract,
            "add_device_from_budget",
            &serde_json::json!({ "token_id": token, "device_id": device }),
        )?;
        Ok(token)
    }
}

/// The platform name of this computer, as the build records use it:
/// `<os>-<arch>` with the Rust names, for example `linux-x86_64`,
/// `windows-x86_64` or `macos-aarch64`.
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
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

/// How long the register page may take: the presence answer lasts this long,
/// and the Bevy plugin waits this long for the seat.
pub const REGISTER_WAIT: Duration = Duration::from_secs(600);

/// Percent-encodes everything except the URL-safe characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn open_url(url: &str) -> Result<(), Error> {
    // Absolute paths on Windows and macOS, so that a program of the same name
    // next to the app or earlier in PATH does not run. Linux desktops put
    // xdg-open in different places, and it only opens the URL.
    let result = if cfg!(target_os = "windows") {
        // Not "cmd /C start": cmd reads each "&" in the URL as a command separator.
        std::process::Command::new(device::system32("rundll32.exe"))
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| Error::Io(format!("cannot open {url}: {e}")))
}

/// The largest download that `download_latest` accepts: 2 GiB.
const MAX_DOWNLOAD: u64 = 2 << 30;

/// The result of [`Client::buy_with_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purchase {
    /// The license token id, for example `checksum:1`.
    pub token_id: String,
    /// False when the account owned the license already and nothing was sent.
    pub new: bool,
}

/// An agent budget, as `get_budget` returns it. NEAR amounts are yoctoNEAR.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct AgentBudget {
    /// The NEAR left, in yoctoNEAR.
    pub balance: String,
    /// The highest price of one license, in yoctoNEAR.
    pub max_price: String,
    /// The products that the budget can buy.
    pub products: Vec<String>,
}

/// A license of the product, as `licenses_for_owner` returns it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OwnedLicense {
    /// The token id.
    pub token_id: String,
    /// The end of the update period (Unix ms).
    pub updates_until_ms: u64,
    /// The computer ids with a seat.
    pub devices: Vec<String>,
}

fn key_account(key: &KeyFile) -> Result<&str, Error> {
    if key.account_id().is_empty() {
        return Err(Error::Io(
            "the key file has no account_id: give the account of the key".into(),
        ));
    }
    Ok(key.account_id())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes `reader` to a new temporary file in `dir`, at most `limit` bytes,
/// and renames it to `name` only when its SHA-256 is `sha256`. Removes the
/// temporary file on every failure.
fn save_checked(
    mut reader: impl std::io::Read,
    dir: &std::path::Path,
    name: &str,
    sha256: &str,
    limit: u64,
) -> Result<std::path::PathBuf, Error> {
    use std::io::Write;
    let mut raw = [0u8; 8];
    getrandom::getrandom(&mut raw).map_err(|e| Error::Io(format!("no random source: {e}")))?;
    let tmp = dir.join(format!(".{name}.{}.part", hex::encode(raw)));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| Error::Io(format!("cannot write {}: {e}", tmp.display())))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > limit {
                return Err(Error::Io(format!(
                    "the download is larger than {limit} bytes"
                )));
            }
            h.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| Error::Io(e.to_string()))?;
        }
        file.sync_all().map_err(|e| Error::Io(e.to_string()))?;
        let got = hex::encode(h.finalize());
        if got != sha256.to_ascii_lowercase() {
            return Err(Error::Io(format!(
                "the download has SHA-256 {got}, not the recorded {sha256}"
            )));
        }
        let path = dir.join(name);
        std::fs::rename(&tmp, &path).map_err(|e| Error::Io(e.to_string()))?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_network_has_its_own_website() {
        assert_eq!(
            Product::testnet("c", "k", "1").site_url,
            "https://testnet.builddeed.dev"
        );
        assert_eq!(
            Product::mainnet("c", "k", "1").site_url,
            "https://builddeed.dev"
        );
    }
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    /// A 64-hex computer id: 6844b8b0, 52 zeros, af75.
    fn dev() -> String {
        format!("6844b8b0{}af75", "0".repeat(52))
    }

    fn client(url: String) -> Client {
        let product =
            Product::testnet("licenses.builddeed.testnet", "doom", "0.2.0").with_rpc_url(url);
        Client::with_device(product, dev())
    }

    // ----- Agents -----

    fn agent_key() -> KeyFile {
        let k = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let mut both = [9u8; 32].to_vec();
        both.extend_from_slice(k.verifying_key().as_bytes());
        KeyFile::parse(
            &serde_json::json!({
                "account_id": "owner.testnet",
                "public_key": format!("ed25519:{}", bs58::encode(k.verifying_key().as_bytes()).into_string()),
                "private_key": format!("ed25519:{}", bs58::encode(&both).into_string()),
            })
            .to_string(),
        )
        .unwrap()
    }

    fn access_key(permission: serde_json::Value) -> (u16, String) {
        fake_rpc::ok(serde_json::json!({
            "nonce": 5, "permission": permission,
            "block_hash": bs58::encode([3u8; 32]).into_string(),
        }))
    }

    fn agent_permission() -> serde_json::Value {
        serde_json::json!({"FunctionCall": {"allowance": "1", "receiver_id": "licenses.builddeed.testnet",
            "method_names": ["buy_from_budget", "add_device_from_budget"]}})
    }

    fn license(token: &str, until: u64, devices: &[String]) -> serde_json::Value {
        serde_json::json!({"token_id": token, "product": "doom", "owner_id": "owner.testnet",
            "purchased_at_ms": 1, "updates_until_ms": until, "listed_price": null, "devices": devices})
    }

    const FUTURE: u64 = 4_000_000_000_000;

    #[test]
    fn buy_sends_nothing_when_a_license_is_owned() {
        let url = fake_rpc::serve(vec![fake_rpc::view(serde_json::json!([license(
            "doom:1",
            FUTURE,
            &[]
        )]))]);
        let p = client(url).buy_with_key(&agent_key()).unwrap();
        assert_eq!(
            p,
            Purchase {
                token_id: "doom:1".into(),
                new: false
            }
        );
    }

    #[test]
    fn buy_passes_the_price_and_returns_the_new_token() {
        let ret = B64.encode(br#"{"token_id":"doom:2"}"#);
        let url = fake_rpc::serve(vec![
            fake_rpc::view(serde_json::json!([license("doom:1", 1, &[])])),
            fake_rpc::view(serde_json::json!({"price": "100"})),
            access_key(agent_permission()),
            fake_rpc::ok(serde_json::json!({"status": {"SuccessValue": ret}})),
        ]);
        let p = client(url).buy_with_key(&agent_key()).unwrap();
        assert_eq!(
            p,
            Purchase {
                token_id: "doom:2".into(),
                new: true
            }
        );
    }

    #[test]
    fn buy_refuses_a_full_access_key() {
        let url = fake_rpc::serve(vec![
            fake_rpc::view(serde_json::json!([])),
            fake_rpc::view(serde_json::json!({"price": "100"})),
            access_key(serde_json::json!("FullAccess")),
        ]);
        let e = client(url).buy_with_key(&agent_key()).unwrap_err();
        assert!(e.to_string().contains("full-access"), "{e}");
    }

    #[test]
    fn a_timed_out_buy_is_unconfirmed_with_its_hash() {
        let url = fake_rpc::serve(vec![
            fake_rpc::view(serde_json::json!([])),
            fake_rpc::view(serde_json::json!({"price": "100"})),
            access_key(agent_permission()),
            fake_rpc::error("TIMEOUT_ERROR"),
        ]);
        match client(url).buy_with_key(&agent_key()) {
            Err(Error::Unconfirmed { tx_hash }) => assert!(tx_hash.len() > 30, "{tx_hash}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn register_returns_the_seat_it_has_and_refuses_another_license() {
        let seated = serde_json::json!([license("doom:1", FUTURE, &[dev()])]);
        let url = fake_rpc::serve(vec![fake_rpc::view(seated.clone())]);
        assert_eq!(
            client(url).register_with_key(&agent_key(), None).unwrap(),
            "doom:1"
        );
        let url = fake_rpc::serve(vec![fake_rpc::view(seated)]);
        let e = client(url)
            .register_with_key(&agent_key(), Some("doom:2"))
            .unwrap_err();
        assert!(e.to_string().contains("doom:1"), "{e}");
    }

    #[test]
    fn register_picks_a_current_license_with_a_free_seat() {
        let full: Vec<String> = (0..3).map(|i| format!("{i:064}")).collect();
        let url = fake_rpc::serve(vec![
            fake_rpc::view(serde_json::json!([
                license("doom:1", FUTURE, &full),
                license("doom:2", 1, &[]),
                license("doom:3", FUTURE, &[]),
            ])),
            fake_rpc::view(serde_json::json!({"max_devices": 3})),
            access_key(agent_permission()),
            fake_rpc::ok(serde_json::json!({"status": {"SuccessValue": ""}})),
        ]);
        assert_eq!(
            client(url).register_with_key(&agent_key(), None).unwrap(),
            "doom:3"
        );
    }

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("builddeed-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sha_of(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    #[test]
    fn a_checked_download_gets_its_name() {
        let d = tmp_dir("ok");
        let p = save_checked(&b"good"[..], &d, "app", &sha_of(b"good"), 100).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"good");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn a_bad_or_broken_download_never_replaces_a_file() {
        struct Broken;
        impl std::io::Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("reset"))
            }
        }
        let d = tmp_dir("bad");
        std::fs::write(d.join("app"), b"good").unwrap();
        assert!(save_checked(&b"evil"[..], &d, "app", &sha_of(b"good"), 100).is_err());
        assert!(save_checked(Broken, &d, "app", &sha_of(b"good"), 100).is_err());
        assert!(save_checked(&b"good"[..], &d, "app", &sha_of(b"good"), 3).is_err());
        assert_eq!(std::fs::read(d.join("app")).unwrap(), b"good");
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            1,
            "no temporary file is left"
        );
    }

    #[test]
    fn urls_name_product_device_and_version() {
        let c = client("http://x".into());
        assert_eq!(
            c.register_url().unwrap(),
            format!(
                "https://testnet.builddeed.dev/register/#product=doom&device={}&version=0.2.0",
                dev()
            )
        );
        assert_eq!(c.buy_url(), "https://testnet.builddeed.dev/p/doom/");
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
        assert!(c.buy_url().ends_with("/p/k/"));
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
    fn short_id_and_urls_are_safe_for_any_input() {
        let c = Client::with_device(Product::testnet("c", "k&x=1", "1.0 beta"), "é");
        assert_eq!(c.short_device_id().unwrap(), "é");
        let u = c.register_url().unwrap();
        assert!(
            u.contains("product=k%26x%3D1&device=%C3%A9&version=1.0%20beta"),
            "{u}"
        );
    }

    #[test]
    fn find_artifact_checks_input_and_answer() {
        let other = serde_json::json!({
            "version": "0.2.0", "commit": "a".repeat(40), "released_at_ms": 5u64,
            "artifact": { "platform": "linux-x86_64", "file": "f", "sha256": "c".repeat(64),
                          "attestation_url": null }
        });
        let url = fake_rpc::serve(vec![fake_rpc::view(other)]);
        let c = client(url);
        assert!(
            matches!(c.find_artifact("xyz"), Err(Error::Io(_))),
            "bad input"
        );
        assert!(
            matches!(c.find_artifact(&"B".repeat(64)), Err(Error::Rpc(m)) if m.contains("another file")),
            "answer for another hash"
        );
    }

    #[test]
    fn plain_http_rpc_is_refused_except_loopback() {
        let c = Client::with_device(
            Product::testnet("c", "k", "1").with_rpc_url("http://rpc.example.com"),
            dev(),
        );
        assert!(matches!(c.check(), Err(Error::Rpc(m)) if m.contains("https")));
    }

    #[test]
    fn mainnet_uses_the_mainnet_endpoint() {
        let p = Product::mainnet("x.builddeed.near", "x", "1.0.0");
        assert_eq!(p.network, Network::Mainnet);
        assert!(p.rpc_url.contains("mainnet"));
    }
}
