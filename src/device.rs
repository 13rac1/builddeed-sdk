//! The computer id: a seat on a license is for one computer.
//!
//! The id is the SHA-256 of the operating system's machine id, the license
//! contract and a fixed label. The app computes it at each start and does
//! not store it, so a copied file does not move a seat to another computer.
//! The hash hides the machine id, and the contract in the hash gives each
//! product a different id for the same computer.

use sha2::{Digest, Sha256};

const LABEL: &str = "builddeed-device-v1";

/// The computer id for a license contract: 64 lowercase hex characters.
pub fn device_id(contract: &str) -> Result<String, String> {
    Ok(id_from(contract, &machine_id()?))
}

fn id_from(contract: &str, machine_id: &str) -> String {
    let h = Sha256::digest(format!("{LABEL}\n{contract}\n{machine_id}").as_bytes());
    hex::encode(h)
}

/// The machine id that the operating system keeps for this installation.
fn machine_id() -> Result<String, String> {
    let id = if cfg!(target_os = "linux") {
        ["/etc/machine-id", "/var/lib/dbus/machine-id"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .map(|s| s.trim().to_string())
    } else if cfg!(target_os = "macos") {
        command_output("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])
            .and_then(|out| parse_ioreg(&out))
    } else if cfg!(target_os = "windows") {
        command_output(
            "reg",
            &[
                "query",
                r"HKLM\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ],
        )
        .and_then(|out| parse_reg(&out))
    } else {
        None
    };
    id.filter(|s| s.len() >= 16)
        .ok_or_else(|| "Cannot read the machine id of this computer".to_string())
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
    fn id_is_hex_and_depends_on_contract_and_machine() {
        let a = id_from("demo.builddeed.testnet", "0123456789abcdef0123456789abcdef");
        assert_eq!(a.len(), 64);
        assert!(a
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(
            a,
            id_from("demo.builddeed.testnet", "0123456789abcdef0123456789abcdef")
        );
        assert_ne!(
            a,
            id_from("other.testnet", "0123456789abcdef0123456789abcdef")
        );
        assert_ne!(
            a,
            id_from("demo.builddeed.testnet", "fedcba9876543210fedcba9876543210")
        );
    }

    #[test]
    fn id_matches_a_fixed_value() {
        // printf 'builddeed-device-v1\nc.testnet\nm' | sha256sum
        assert_eq!(
            id_from("c.testnet", "m"),
            "d0df87748868c2490de8dbf477ba0c8993f7ae9c7d2199e3216551a277480be9"
        );
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
            assert_eq!(device_id("demo.builddeed.testnet").unwrap().len(), 64);
        }
    }
}
