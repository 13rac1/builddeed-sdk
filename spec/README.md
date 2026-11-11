# BuildDeed client protocol

What every BuildDeed SDK does, in any language. The files in
[`../fixtures`](../fixtures) are the test data that each SDK must pass.

## Product

A product is one license contract on NEAR plus the program's version.

| Field | Example | Note |
| --- | --- | --- |
| `contract` | `doom.builddeed.testnet` | The account of the license contract. |
| `key` | `doom` | The product key on the BuildDeed website. |
| `version` | `0.2.0` | The version of the running program, as the release records it with `add_build`. |
| `rpc_url` | `https://rpc.testnet.fastnear.com` | A NEAR JSON-RPC endpoint of the contract's network. |
| `site_url` | `https://13rac1.github.io/builddeed` | The BuildDeed website, no trailing slash. |

## Computer id

```
device_id = lowercase hex( SHA-256( "builddeed-device-v1" + "\n" + contract + "\n" + machine_id ) )
```

- `machine_id` is the operating system's id of the installation, as text,
  with surrounding white space removed:
  - Linux: the content of `/etc/machine-id`, else `/var/lib/dbus/machine-id`.
  - macOS: `IOPlatformUUID` from `ioreg -rd1 -c IOPlatformExpertDevice`.
  - Windows: `MachineGuid` from
    `HKLM\SOFTWARE\Microsoft\Cryptography` (`reg query ... /v MachineGuid`).
- A machine id shorter than 16 characters is not accepted: the computer has
  no id.
- Compute the id at each start. Never store it: a stored id would move a seat
  with a copied file.
- Test data: [`fixtures/device-id.json`](../fixtures/device-id.json).

## The license check

One view call at each start:

```json
{"jsonrpc":"2.0","id":"builddeed","method":"query","params":{
  "request_type":"call_function","finality":"final",
  "account_id":"<contract>","method_name":"device_license",
  "args_base64":"<base64 of {\"device_id\":\"<device_id>\",\"version\":\"<version>\"}>"}}
```

The view result is the bytes of a JSON value:

| Result | Meaning | The program |
| --- | --- | --- |
| `{"token_id", "owner_id", "updates_until_ms", "covers_version": true}` | A seat on a license that covers this version | Registered: turn the feature on. |
| `{..., "covers_version": false}` | A seat, but the license ended before this version | Unregistered; tell the user to renew. |
| `null` | No seat | Unregistered. |
| An RPC error or no network | No answer | Unregistered; show the error. |

Classify errors by `error.cause.name`, not by text: `UNKNOWN_ACCOUNT` and
`UNKNOWN_ACCESS_KEY` mean "does not exist"; anything else, an HTTP error
without a JSON body, or no connection is a failure. Test data:
[`fixtures/device-license.json`](../fixtures/device-license.json).

**No offline mode.** Do not store a license result on the computer: it could
be copied or edited. With no network, the program runs unregistered.

## Registration

1. Open `register_url` in the browser:
   `<site_url>/register.html#product=<key>&device=<device_id>&version=<version>`
2. The owner registers the computer on the website (`add_device`, one wallet
   transaction).
3. Ask `device_license` again every few seconds (5 s) until a seat that covers
   the version is there, the user stops, or a time limit (10 minutes) ends.
   Retry on RPC errors; stop on "license ended".

The buy page is `<site_url>/buy.html?product=<key>`. On Windows, open URLs
with `rundll32 url.dll,FileProtocolHandler <url>`, not `cmd /C start` (cmd
reads `&` as a command separator). Test data:
[`fixtures/urls.json`](../fixtures/urls.json).

## Build records

`find_artifact({"sha256": "<64 hex>"})` returns the official build that
contains a file, or `null`:

```json
{"version":"0.2.0","commit":"<40 hex>","released_at_ms":0,
 "artifact":{"platform":"linux-x86_64","file":"...","sha256":"...","attestation_url":"https://..."}}
```

`verify_self` hashes the running program (read in blocks) and calls
`find_artifact`. `null` means the contract has no record: for example a
self-built copy.
