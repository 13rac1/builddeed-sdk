# BuildDeed SDK

Sell a Rust program with [BuildDeed](https://github.com/13rac1/builddeed):
software licenses as NFTs on NEAR. A license has three computer seats, each
sale pays the developer and the contributors in the contract, and the
contract records the SHA-256 of every official build.

## Use it

```toml
[dependencies]
builddeed = { git = "https://github.com/13rac1/builddeed-sdk", tag = "rust-v0.1.1" }
```

```rust
use builddeed::{Client, Product};

let client = Client::new(Product::testnet(
    "licenses.builddeed.testnet", // the official BuildDeed contract
    "doom",                   // your product key on the BuildDeed website
    env!("CARGO_PKG_VERSION"),
));
match client.check() {
    Ok(Some(seat)) => println!("Registered to {}", seat.account_id), // turn the feature on
    Ok(None) => { client.open_register_page().ok(); }                 // nag; offer to register
    Err(e) => println!("Running unregistered: {e}"),                  // no network
}
```

After `open_register_page`, call `wait_for_seat` in a worker thread: it asks
the contract every few seconds until the owner registers the computer on the
website.

| Call | What it does |
| --- | --- |
| `Client::new(product)` | Computes this computer's id: SHA-256 of the OS machine id, the contract name and the product key. |
| `check()` | The seat of this computer on a license that covers this version, or `None`. One view call. |
| `status()` | Like `check()`, and tells "license ended, renew" apart from "no seat". |
| `device_id_from(contract, product, machine_id)` | The computer id rule of the spec, for tests and tools. |
| `register_url()`, `open_register_page()` | The BuildDeed register page for this computer; `open_register_page` adds the presence check. |
| `confirm_code()` | The 6-digit code to show while the register page is open. The page asks for it when the browser blocks the presence check. |
| `buy_url()`, `open_buy_page()` | The product page, where a user buys (`<site_url>/p/<key>/`). |
| `wait_for_seat(stop, every, max)` | Polls until the seat is there, `stop` is set, or `max` passes. |
| `find_artifact(sha256)` | The official build record of a file. |
| `verify_self()` | The record of the running program; `None` for a self-built copy. |
| `latest_build(platform)`, `download_latest(platform, dir)` | The last recorded official file for a platform. The download goes to a temporary name and gets its name only when its SHA-256 is the recorded one. |
| `KeyFile::load(path)`, `buy_with_key(key)`, `register_with_key(key, token)`, `budget(account)`, `licenses_of(account)` | Buy and register with an agent key and the owner's agent budget, with no browser. `buy_with_key` sends nothing when the account owns a license of the product whose update period has not ended. A key that is not an agent key for the contract is refused. |

### Command line, for agents and CI

```sh
cargo install --locked --git https://github.com/13rac1/builddeed-sdk --tag rust-v0.1.1 builddeed --bin builddeed
builddeed --product checksum budget --key key.json     # the budget and the licenses
builddeed --product checksum buy --key key.json        # {"token_id": "checksum:1", "bought": true, ...}
builddeed --product checksum register --key key.json   # a seat for THIS computer
builddeed --product checksum download --out .          # the official file, SHA-256 checked
builddeed --product checksum status
```

Every command prints one JSON object and exits with 0, or 1 and `{"error": ...}`.
After a sent transaction with an unknown result, the error has `unconfirmed_tx`:
check `budget` before you run the command again. `--help` prints the commands.

`--key` takes a NEAR key file. A file in `~/.near-credentials/<network>/<account>/`
names its account by the folder; for a key file elsewhere, add `--account <id>`.
Give the agent a function-call key, never a full-access key: see "Agents and
programs with no browser" in the [spec](../spec/README.md).

## Features

| Feature | Content |
| --- | --- |
| `bevy` | `bevy_plugin::BuildDeedPlugin`: a `License` resource, `RegisterRequest` and `BuyRequest` events. |
| `test-support` | `fake_rpc`: a loopback RPC server with fixed replies, to test your gating offline. |

## Rules that the SDK follows

- No offline mode: a license result stored on the computer could be copied or
  edited, so the SDK stores none. With no network, run unregistered.
- The computer id is computed at each start and never stored, so a copied
  app file does not move a seat. A user with root or administrator rights can
  set the machine id, and computers cloned from one disk image share an id:
  the seat limit does not hold against a copied machine id.
- `open_register_page` runs the presence check: for 10 minutes the program
  answers a one-time code on 127.0.0.1, and the website asks the owner's
  wallet only when the answer names this computer. When the browser blocks
  the check, the page asks for `confirm_code()`: show it in your waiting
  screen.
- RPC endpoints must use https (loopback excepted). `find_artifact` checks
  that the answer is for the file it asked about. `verify_self` is not a
  security check: a changed program controls its own answer.
- The check is in your public source code. Anyone may build your program
  without it; the official build is the one that your contract records.

## Used by

- [BuildDeed Checksum](https://github.com/13rac1/builddeed/tree/main/app) (Slint)
- [Room4Doom](https://github.com/ai-13rac1/room4doom) (`Client` directly, no Bevy)

## Test

```sh
cargo test --all-features
```

## License

Apache License 2.0. See [LICENSE](../LICENSE). The protocol is in [spec/](../spec/README.md).
