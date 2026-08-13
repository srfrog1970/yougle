# pm-transport

iroh endpoint management and the client side of the `pm-node` wire protocol.

Status: M2 client half complete.

- `NodeClient` — binds an iroh `Endpoint` and makes one-request-per-connection calls to a Server mailbox: open a bidirectional QUIC stream, write the serialized `NodeRequest`, finish the send side, read the `NodeResponse` to end.
- Stands in for `pm-core` (which doesn't exist yet) in the M2 integration test in `pm-node/tests/`.

Not yet built: signed pointer-record publication/resolution for Local-to-Server mailbox switching (`docs/PRD.md` §8's "Signed mailbox-pointer updates" requirement) — that's a later milestone, since it needs pairing (which needs `pm-core`) to exist first.

## Running the tests locally

```
cargo test -p pm-transport
cargo clippy -p pm-transport --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
