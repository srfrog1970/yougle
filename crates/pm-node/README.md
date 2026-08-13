# pm-node

The Server mailbox binary. Per the PRD (v2.0), this is single-tenant, self-hosted infrastructure a user runs for themselves (e.g., on a Raspberry Pi) — not shared community infrastructure.

Status: M2 complete (v0 — in-memory only, per `ARCHIT_1.MD` §7's M2 scope; persistence is later).

- `store` — the in-memory mailbox: a set of registered write-authorization hashes, and the blobs deposited against them. `write` verifies `SHA256(auth)` against the registered set and consumes it (one-time use); `ack` marks delivered without deleting, per `docs/PRD.md`'s retention requirement.
- `handler` — implements iroh's `ProtocolHandler`, dispatching `NodeRequest`s (`RegisterSlot`/`Write`/`Fetch`/`Ack`, defined in `pm_proto::node_protocol`) to the store, gating owner-only calls (`Fetch`/`Ack`/`RegisterSlot`) by a configured `mailbox_key`.
- `main.rs` — the binary: reads `PM_NODE_MAILBOX_KEY` (hex) from the environment, binds, prints its endpoint address, and runs until `Ctrl-C`.

**Deliberately simplified from `ARCHIT_1.MD`'s original node API** (`REDEEM_TICKET`, slots padded to a fixed 4096 count regardless of real usage). That padding existed to hide contact-graph information from a shared, third-party-operated community mailbox — `docs/PRD.md` v2.0 removed community mailboxes entirely, so there's no one left to hide from. What's kept is the part that doesn't depend on that: unforgeable write authorization via a pre-registered hash. Whether something closer to the original anti-enumeration scheme is still warranted for some other reason is an open item in `docs/PRD.md` and `docs/THREAT-MODEL.md`, not resolved here.

`tests/m2_exit_criteria.rs` covers M2's exit criteria: two client identities (via `pm-transport`, standing in for `pm-core`, which doesn't exist yet) exchange a real Olm-encrypted, `pm-proto`-framed message through a real running node over actual iroh QUIC connections — register, write, fetch, decrypt, verify the signature, ack — while directly inspecting the node's own storage to confirm it only ever held opaque, auth-gated bytes, never plaintext.

## Running the node

```
PM_NODE_MAILBOX_KEY=<64 hex chars> cargo run -p pm-node
```

## Running the tests locally

```
cargo test -p pm-node
cargo clippy -p pm-node --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
