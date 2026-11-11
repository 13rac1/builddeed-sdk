# BuildDeed client protocol

What every BuildDeed SDK does, in any language. The files in
[`../fixtures`](../fixtures) are the test data that each SDK must pass.

## Product

A product is a key in a license contract on NEAR, plus the program's
version. One contract holds many products, and all of their licenses are one
NFT collection. The official BuildDeed collection on testnet is
`licenses.builddeed.testnet`.

| Field | Example | Note |
| --- | --- | --- |
| `contract` | `licenses.builddeed.testnet` | The account of the license contract. |
| `key` | `doom` | The product key in the contract (`add_product`) and on the website. |
| `version` | `0.2.0` | The version of the running program, as the release records it with `add_build`. |
| `rpc_url` | `https://rpc.testnet.fastnear.com` | A NEAR JSON-RPC endpoint of the contract's network. |
| `site_url` | `https://13rac1.github.io/builddeed` | The BuildDeed website, no trailing slash. |

## Computer id

```
device_id = lowercase hex( SHA-256( "builddeed-device-v1" + "\n" + contract + "\n" + key + "\n" + machine_id ) )
```

- `machine_id` is the operating system's id of the installation, as text,
  with surrounding white space removed:
  - Linux: the first usable value of `/etc/machine-id`, then
    `/var/lib/dbus/machine-id`.
  - macOS: `IOPlatformUUID` from `/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice`.
  - Windows: `MachineGuid` from `%SystemRoot%\System32\reg.exe query
    HKLM\SOFTWARE\Microsoft\Cryptography /v MachineGuid /reg:64` (`/reg:64`:
    a 32-bit program reads the 64-bit registry view, where the value is).
- Call helper programs by absolute path, so that a program of the same name
  earlier in `PATH` or next to the app cannot answer.
- A machine id that is shorter than 16 characters, or is `uninitialized`, is
  not usable. If no source gives a usable id, the computer has no id and the
  program runs unregistered.
- Compute the id at each start. Never store it: a stored id would move a seat
  with a copied app file.
- Limit: a user with root or administrator rights can set the machine id. A
  machine id copied from a computer with a seat registers another computer,
  and computers cloned from one disk image share an id. The seat limit holds
  against copied app files, not against a copied machine id.
- Test data: [`fixtures/device-id.json`](../fixtures/device-id.json).

## The license check

One view call at each start:

```json
{"jsonrpc":"2.0","id":"builddeed","method":"query","params":{
  "request_type":"call_function","finality":"final",
  "account_id":"<contract>","method_name":"device_license",
  "args_base64":"<base64 of {\"product\":\"<key>\",\"device_id\":\"<device_id>\",\"version\":\"<version>\"}>"}}
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

1. Start the presence listener (below), then open the register page:
   `<site_url>/register.html#product=<key>&device=<device_id>&version=<version>&port=<port>&code=<code>`
   Percent-encode each value.
2. The page checks the presence of the program, then the owner registers the
   computer (`add_device`, one wallet transaction).
3. Ask `device_license` again every few seconds (5 s) until a seat that covers
   the version is there, the user stops, or a time limit (10 minutes) ends.
   Retry on RPC errors; stop on "license ended".

### Presence check

A register link carries a computer id, so a link from someone else could
give a seat to their computer. The page therefore asks the program on this
computer first:

- The program listens on `127.0.0.1` on a free port for 10 minutes and makes
  a random one-time `code` (16 bytes, hex).
- `GET /builddeed/<code>` answers `{"contract": "<contract>", "device_id": "<device_id>"}`.
  Other paths answer 404.
- Every answer has `Access-Control-Allow-Origin: <origin of site_url>` (only
  the website may read it), `Access-Control-Allow-Private-Network: true`, and
  `Cache-Control: no-store`. `OPTIONS` answers 204 with the same headers.
- Read at most 8 KiB of a request line.

The page fetches `http://127.0.0.1:<port>/builddeed/<code>`. It enables the
"Register this computer" button only when the answer names the `device` and
the contract of the link. When the answer names another computer, it refuses.
When the browser blocks the request (some browsers ask the user first, others
do not allow a public page to reach 127.0.0.1), the page asks for the
confirmation code (below).

### Confirmation code

While the register page is open, the program shows a 6-digit code with the
text "If the page asks for a code, enter this one":

```
confirm_code = decimal( first 4 bytes of SHA-256( "builddeed-confirm-v1" + "\n" + device_id ), big-endian ) mod 1000000
```

Write it with 6 digits (leading zeros). The page computes the same code from
the `device` of the link and asks for it only when the presence check was
blocked or the link has no `port` and `code`. A link for another computer
expects that computer's code, and the program here shows another one, so
the page refuses.

Limit: the code comes from the computer id, which is in the link. A person
who sends a link for their computer can also send its code. The code stops
an owner who clicks through without looking at the program; it does not
stop an owner who types a code from a message. Only the presence check
proves that the program is on this computer. Test data:
[`fixtures/confirm-code.json`](../fixtures/confirm-code.json).

The buy page is `<site_url>/buy.html?product=<key>`. On Windows, open URLs
with `rundll32 url.dll,FileProtocolHandler <url>`, not `cmd /C start` (cmd
reads `&` as a command separator). Test data:
[`fixtures/urls.json`](../fixtures/urls.json).

## Build records

`find_artifact({"product": "<key>", "sha256": "<64 hex>"})` returns the
official build of the product that contains a file, or `null`. Refuse an input that is not 64 hex characters,
and an answer whose `artifact.sha256` is not the one asked for:

```json
{"version":"0.2.0","commit":"<40 hex>","released_at_ms":0,
 "artifact":{"platform":"linux-x86_64","file":"...","sha256":"...","attestation_url":"https://..."}}
```

`verify_self` hashes the running program (read in blocks) and calls
`find_artifact`. `null` means the contract has no record: for example a
self-built copy. It is not a security check: a changed program controls its
own answer. Check a download from outside the program.

## RPC

Use `https` RPC endpoints only; refuse plain `http` except for loopback
(tests, local nodes).
