# BuildDeed SDKs

Libraries for programs sold with [BuildDeed](https://github.com/13rac1/builddeed):
software licenses as NFTs on NEAR, with three computer seats per license and
a record of official builds in the license contract.

| Folder | What it is |
| --- | --- |
| [`spec/`](spec/README.md) | The client protocol: computer id, license check, registration, build records. |
| [`fixtures/`](fixtures) | Test data that every SDK must pass: computer-id and confirmation-code vectors, RPC replies, URLs. |
| [`rust/`](rust/README.md) | The Rust crate `builddeed` (feature `bevy` for a Bevy plugin). |
| [`go/`](go/README.md) | The Go module `github.com/13rac1/builddeed-sdk/go`, standard library only. |

Each SDK has its own version and tags: `rust-v0.1.0` for the Rust crate,
`go/v0.1.0` for the Go module.

## Used by

- [BuildDeed Checksum](https://github.com/13rac1/builddeed/tree/main/app) (Rust, egui)
- [Room4Doom](https://github.com/ai-13rac1/room4doom) (Rust, a Doom engine; uses `Client` directly)

## License

Apache License 2.0. See [LICENSE](LICENSE).
