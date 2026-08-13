# pm-proto

Envelopes, chain derivation, and wire-format versioning. Shared verbatim between `pm-core` (client) and `pm-node` (server mailbox binary) so framing and derivation logic can never drift between the two.

Status: M0 in progress.

- `envelope` — the `Envelope` struct (version, olm_type placeholder, ciphertext, chain index, Lamport clock, timestamp, message id, reserved attachments field) plus fixed-bucket padding so serialized size doesn't leak message length.
- `padding` — the 256 B / 1 KB / 4 KB bucket scheme, with property tests confirming padded length never reveals input size within a bucket.
- `derive` — the generic HKDF-SHA256 chain-derivation primitive (`derive(key, label, n, len)`), with a pinned test vector and property tests for determinism and output distinctness.
- `error` — shared error type.

No networking, storage, or crypto-session logic lives here — see `pm-crypto`, `pm-store`, `pm-transport` (not yet started).

The original slot-hash/tag-chain padding scheme was designed to hide contact-graph information from a shared community mailbox operator. Since the PRD (v2.0) removed community mailboxes in favor of per-user Local/Server mailboxes, whether the full scheme (as opposed to the `derive` primitive it was built on, which is implemented here) is still needed in its original form is an open question — see `docs/PRD.md`, Open Items.

## Running the tests locally

```
cargo test -p pm-proto
cargo clippy -p pm-proto --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
