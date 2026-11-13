# BuildDeed SDK for Go

Sell a Go program with [BuildDeed](https://github.com/13rac1/builddeed):
software licenses as NFTs on NEAR. A license has three computer seats, each
sale pays the developer and the contributors in the contract, and the
contract records the SHA-256 of every official build.

The module uses the Go standard library only.

## Use it

```sh
go get github.com/13rac1/builddeed-sdk/go@v0.1.2
```

The tag in the repository is `go/v0.1.2`; the go command adds the `go/`
folder to the version.

The package name is `builddeed`:

```go
import builddeed "github.com/13rac1/builddeed-sdk/go"

client := builddeed.New(builddeed.Testnet(
	builddeed.OfficialTestnetContract, // the official BuildDeed contract
	"doom",                            // your product key on the BuildDeed website
	version,                           // the version of this program, as the release records it
))
lic, err := client.Check(ctx)
switch {
case err != nil:
	log.Printf("running unregistered: %v", err) // no network, or no machine id
case lic.Status == builddeed.Registered:
	log.Printf("registered to %s", lic.AccountID) // turn the feature on
case lic.Status == builddeed.LicenseEnded:
	log.Printf("license %s ended before this version: renew it", lic.TokenID)
}
```

To register the computer, open the register page, show the confirmation
code, and wait for the seat in a goroutine. One context controls the
presence listener and the wait: it lives while the page is open, and you
cancel it when the user stops.

```go
ctx, cancel := context.WithTimeout(context.Background(), builddeed.RegisterWait)
defer cancel()
if err := client.OpenRegisterPage(ctx); err != nil {
	log.Printf("cannot open the register page: %v", err)
	return
}
if code, err := client.ConfirmCode(); err == nil {
	fmt.Printf("If the page asks for a code, enter this one: %s\n", code)
}
lic, err = client.WaitForSeat(ctx, builddeed.PollInterval) // context.Canceled: the user stopped
```

| Call | What it does |
| --- | --- |
| `Testnet(contract, key, version)`, `Mainnet(...)` | A product with the default RPC endpoint and website of the network. |
| `New(product)` | Computes the id of this computer: SHA-256 of the OS machine id, the contract name and the product key. |
| `Check(ctx)` | The license of this computer: `Registered`, `LicenseEnded` (renew) or `NoSeat`. One view call. |
| `DeviceIDFor(contract, key, machineID)` | The computer id rule of the spec, for tests and tools. |
| `RegisterURL()`, `OpenRegisterPage(ctx)` | The BuildDeed register page for this computer; `OpenRegisterPage` adds the presence check. |
| `ConfirmCode()` | The 6-digit code to show while the register page is open. The page asks for it when the browser blocks the presence check. |
| `BuyURL()`, `OpenBuyPage()` | The buy page of the product. |
| `WaitForSeat(ctx, interval)` | Asks until the seat is there or `ctx` ends. Stops at once on a wrong contract name (`ErrNotExist`). |
| `FindArtifact(ctx, sha256)` | The official build record of a file. |
| `VerifySelf(ctx)` | The record of the running program; nil for a copy that someone built. |
| `LatestBuild(ctx, platform)`, `DownloadLatest(ctx, platform, dir)` | The last recorded official file for a platform. The download goes to a temporary name and gets its name only when its SHA-256 is the recorded one. |
| `LoadKeyFile(path)`, `BuyWithKey(ctx, key)`, `RegisterWithKey(ctx, key, tokenID)`, `Budget(ctx, account)`, `Licenses(ctx, account)` | Buy and register with an agent key and the owner's agent budget, with no browser. `BuyWithKey` sends nothing when the account owns a license of the product whose update period has not ended. A key that is not an agent key for the contract is refused. |

### Errors

- `ErrNoMachineID`: the computer has no id (often a container or a cloud
  sandbox). Run unregistered. The error text names the cause.
- `*RPCError`: the RPC endpoint answered with an error or an unexpected
  reply. `errors.Is(err, ErrNotExist)` is true when the account or the access
  key does not exist. A failed connection is another error.
- `*UnconfirmedError`: a transaction was sent, but its result is unknown: a
  time-out, a lost connection, or a reply that cannot be read. It can have
  run: read the result (`Licenses`, `Check`) before you send it again.
  `TxHash` is the transaction hash. Only a refusal of the node
  (`INVALID_TRANSACTION`, `PARSE_ERROR`) or a contract panic is a sure
  failure.
- `ErrNoAccount`: the key file names no account; set `KeyFile.AccountID`.

### Agents

Give the agent a function-call key, never a full-access key: see "Agents and
programs with no browser" in the [spec](../spec/README.md). A key file in
`~/.near-credentials/<network>/<account>/` names its account by the folder;
for a key file elsewhere, set `KeyFile.AccountID`.

Use one agent per key. `BuyWithKey` reads the owned licenses before it buys,
so two programs that use one key at the same time can both buy a license. The Rust crate has a
command line program for agents: see [rust/](../rust/README.md).

## Rules that the SDK follows

- No offline mode: a license result stored on the computer could be copied or
  edited, so the SDK stores none. With no network, run unregistered.
- The computer id is computed at each start and never stored, so a copied
  program file does not move a seat. A user with root or administrator rights
  can set the machine id, and computers cloned from one disk image share an
  id: the seat limit does not hold against a copied machine id.
- `OpenRegisterPage` runs the presence check: for 10 minutes the program
  answers a one-time code on 127.0.0.1, and the website asks the owner's
  wallet only when the answer names this computer. When the browser blocks
  the check, the page asks for `ConfirmCode()`: show it in your waiting
  screen.
- RPC endpoints must use https (loopback excepted), and the client follows no
  redirects. `FindArtifact` checks that the answer is for the file it asked
  about. `VerifySelf` is not a security check: a changed program controls its
  own answer.
- The check is in your public source code. Anyone can build your program
  without it; the official build is the one that your contract records.

## TinyGo

Tested with TinyGo 0.44.0 (Go 1.27.3):

- The whole package compiles for Linux, Windows, macOS and `wasip1`.
- The parts with no network work: the computer id, the confirmation code,
  the URLs, key files, transaction encoding and signing, and the checked save
  of a download.
- The network parts do not work: TinyGo's `net` package on Linux cannot
  connect to the RPC endpoint or start the presence listener. `Check`,
  `WaitForSeat`, `OpenRegisterPage`, the build records, the download and the
  agent calls return errors.

## Test

The module needs Go 1.26 or later. CI tests Go 1.26 and 1.27.

```sh
go test -race ./...
```

The tests use the shared data in [`../fixtures`](../fixtures) and a loopback
RPC server; they do not call a NEAR network.

## License

Apache License 2.0. See [LICENSE](../LICENSE). The protocol is in [spec/](../spec/README.md).
