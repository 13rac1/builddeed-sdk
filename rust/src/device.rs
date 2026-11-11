//! The computer id: a seat on a license is for one computer.
//!
//! The id is the SHA-256 of the operating system's machine id, the license
//! contract, the product key and a fixed label. The app computes it at each
//! start and does not store it, so a copied app file does not move a seat to
//! another computer. The hash hides the machine id, and the contract and
//! product in the hash give each product a different id for the same
//! computer.
//!
//! Limit: a user with root or administrator rights can set the machine id of
//! a computer, so a machine id copied from a computer with a seat registers
//! another computer. The helper programs are called by absolute path, so
//! that a program of the same name earlier in PATH, or next to the app,
//! cannot answer in their place.

use sha2::{Digest, Sha256};

const LABEL: &str = "builddeed-device-v1";

/// The computer id for a product of a license contract: 64 lowercase hex
/// characters.
pub fn device_id(contract: &str, product: &str) -> Result<String, String> {
    Ok(id_from(contract, product, &machine_id()?))
}

pub(crate) fn id_from(contract: &str, product: &str, machine_id: &str) -> String {
    let h = Sha256::digest(format!("{LABEL}\n{contract}\n{product}\n{machine_id}").as_bytes());
    hex::encode(h)
}

/// A machine id must have at least this many characters.
const MIN_LEN: usize = 16;

/// True for a machine id that the computer can use: long enough, and not the
/// placeholder of an image that systemd did not set up yet.
fn usable(id: &str) -> bool {
    id.len() >= MIN_LEN && id != "uninitialized"
}

/// The machine id that the operating system keeps for this installation.
fn machine_id() -> Result<String, String> {
    let id = if cfg!(target_os = "linux") {
        // The first file with a usable id: an empty or "uninitialized"
        // /etc/machine-id falls back to the D-Bus copy.
        ["/etc/machine-id", "/var/lib/dbus/machine-id"]
            .iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
            .find(|s| usable(s))
    } else if cfg!(target_os = "macos") {
        command_output("/usr/sbin/ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])
            .and_then(|out| parse_ioreg(&out))
    } else if cfg!(target_os = "windows") {
        // /reg:64: a 32-bit program must read the 64-bit registry view, where
        // MachineGuid is.
        command_output(
            &system32("reg.exe"),
            &[
                "query",
                r"HKLM\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
                "/reg:64",
            ],
        )
        .and_then(|out| parse_reg(&out))
    } else {
        None
    };
    id.filter(|s| usable(s))
        .ok_or_else(|| "Cannot read the machine id of this computer".to_string())
}

/// The absolute path of a program in Windows' System32 folder.
pub(crate) fn system32(program: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\{program}")
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `"IOPlatformUUID" = "564D8F2A-..."` in the output of ioreg.
fn parse_ioreg(out: &str) -> Option<String> {
    let line = out.lines().find(|l| l.contains("\"IOPlatformUUID\""))?;
    let value = line.split('=').nth(1)?.trim().trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

/// `MachineGuid    REG_SZ    3f2504e0-...` in the output of reg query.
fn parse_reg(out: &str) -> Option<String> {
    let line = out
        .lines()
        .find(|l| l.trim_start().starts_with("MachineGuid"))?;
    line.split_whitespace().nth(2).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_hex_and_depends_on_contract_product_and_machine() {
        let m = "0123456789abcdef0123456789abcdef";
        let a = id_from("licenses.builddeed.testnet", "doom", m);
        assert_eq!(a.len(), 64);
        assert!(a
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(a, id_from("licenses.builddeed.testnet", "doom", m));
        assert_ne!(a, id_from("other.testnet", "doom", m));
        assert_ne!(a, id_from("licenses.builddeed.testnet", "checksum", m));
        assert_ne!(
            a,
            id_from(
                "licenses.builddeed.testnet",
                "doom",
                "fedcba9876543210fedcba9876543210"
            )
        );
    }

    #[test]
    fn id_matches_a_fixed_value() {
        // printf 'builddeed-device-v1\nc.testnet\np\nm' | sha256sum
        assert_eq!(
            id_from("c.testnet", "p", "m"),
            "2c5f892217c817ffa5f7acd469abed07a426cbd850cd240c9d47841e0d2953b0"
        );
    }

    #[test]
    fn unusable_machine_ids_are_refused() {
        assert!(!usable(""));
        assert!(!usable("uninitialized"));
        assert!(!usable("0123456789abcde"));
        assert!(usable("0123456789abcdef"));
    }

    #[test]
    fn system32_is_an_absolute_windows_path() {
        assert!(system32("reg.exe").ends_with(r"\System32\reg.exe"));
    }

    #[test]
    fn parses_ioreg_output() {
        let out = "+-o J314sAP  <class IOPlatformExpertDevice>\n    {\n      \"IOPlatformSerialNumber\" = \"C02XX\"\n      \"IOPlatformUUID\" = \"564D8F2A-1B2C-4D5E-8F90-A1B2C3D4E5F6\"\n    }\n";
        assert_eq!(
            parse_ioreg(out).as_deref(),
            Some("564D8F2A-1B2C-4D5E-8F90-A1B2C3D4E5F6")
        );
        assert_eq!(parse_ioreg("nothing"), None);
    }

    #[test]
    fn parses_reg_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Cryptography\r\n    MachineGuid    REG_SZ    3f2504e0-4f89-11d3-9a0c-0305e82c3301\r\n\r\n";
        assert_eq!(
            parse_reg(out).as_deref(),
            Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301")
        );
        assert_eq!(parse_reg("nothing"), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn this_computer_has_an_id() {
        // The build container has /etc/machine-id only if the image has one.
        if std::path::Path::new("/etc/machine-id").exists() {
            assert_eq!(
                device_id("licenses.builddeed.testnet", "doom")
                    .unwrap()
                    .len(),
                64
            );
        }
    }
}
