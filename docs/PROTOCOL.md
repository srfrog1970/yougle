# Protocol

Wire format specification: envelopes, padding, key derivation, the two peer-to-peer wire protocols (Server-mailbox and direct), signed mailbox-pointer updates, and backup encryption. Describes what's actually implemented in `pm-proto`/`pm-crypto`/`pm-core`/`pm-node` today, not an aspirational design — see each section's own note on what isn't built yet.

## 1. Envelope

The unit every message-bearing wire exchange carries — `pm_proto::Envelope` (`crates/pm-proto/src/envelope.rs`):

| Field | Type | Notes |
|---|---|---|
| `version` | `u8` | Currently always `1` (`ENVELOPE_VERSION`). See §8, Versioning. |
| `olm_type` | `u8` | vodozemac's prekey-vs-normal message discriminant. |
| `ciphertext` | `Vec<u8>` | Olm-encrypted `PlaintextPayload` (see §5). |
| `n` | `u64` | Position in the sender's per-direction write sequence. |
| `lamport` | `u64` | Lamport clock value, for cross-device/cross-mailbox message ordering. |
| `sent_at` | `u64` | Unix milliseconds. |
| `msg_id` | `[u8; 16]` | Unique per message; ties a `Receipt` back to the original `Chat`. |
| `attachments` | `Vec<AttachmentRef>` | Always empty — attachments are out of scope for the MVP (`docs/PRD.md` §3). `AttachmentRef { id: [u8; 16], size: u64 }`. |

`Envelope`s never travel raw — always through `to_padded_bytes()`/`from_padded_bytes()` (bincode serialize/deserialize, wrapped by padding — see §2), and the `ciphertext` field inside is itself the output of encrypting a bincode-serialized `PlaintextPayload` (§5), not raw chat text.

## 2. Padding

`pm_proto::padding` (`crates/pm-proto/src/padding.rs`) pads a serialized `Envelope` before it's handed to any transport, so the wire length doesn't leak the plaintext message's length to anyone observing ciphertext size (a mailbox operator, a network observer) — see `docs/THREAT-MODEL.md` for what this does and doesn't cover.

Three fixed buckets: **256, 1024, 4096 bytes.** A 4-byte big-endian length prefix is written ahead of the real data, then the buffer is zero-filled up to the smallest bucket that fits (`prefix + data`). Maximum payload size is therefore 4092 bytes (4096 − 4); anything larger fails to pad (`ProtoError::TooLargeToPad`) rather than silently truncating.

`unpad` reverses this: reads the 4-byte prefix, validates it doesn't overrun the buffer, and slices out exactly that many bytes.

## 3. Key derivation

Two independent derivation mechanisms exist, serving different layers:

**Per-user identity keys** — `pm_crypto::Identity::derive(seed)` (`crates/pm-crypto/src/identity.rs`), six keys, each `HKDF-SHA256(seed, label)` with no salt, each label domain-separated and pairwise distinct:

| Key | Label | Used for |
|---|---|---|
| `signing_key` | `pm-identity-v1` | Ed25519 signing key — pairing payloads, mailbox-pointer updates (§6). |
| `backup_key` | `pm-backup-v1` | Backup export/import encryption (§7). |
| `mailbox_key` | `pm-mailbox-v1` | Dual-purpose: mailbox-ownership auth token in the node protocol (§4) *and* the local SQLCipher database key (§8). |
| `backup_location_key` | `pm-backuploc-v1` | Derived but currently unused anywhere — reserved for a future automatic-backup-location feature. |
| `transport_key` | `pm-transport-v1` | Seeds this device's own iroh identity for Local-delivery (§5). |
| `server_transport_key` | `pm-transport-server-v1` | Seeds a self-hosted `pm-node`'s iroh identity, if this user runs one — deliberately a separate identity from `transport_key` so a phone and its owner's own server never claim the same network identity simultaneously. |

**In-protocol derivation** — `pm_proto::derive(key, label, n, out_len)` (`crates/pm-proto/src/derive.rs`): `HKDF-SHA256(key).expand(label_bytes || n.to_be_bytes(), out_len)`. Two call sites, two labels, both in `pm-core`:

- `"pairing"` — derives the shared `pair_secret` from two pairing nonces (§4).
- `"auth"` — derives each per-write authorization value from `pair_secret` and a monotonic counter `n` (§6).

## 4. Pairing

Two devices exchange a `PairingPayload` (`crates/pm-core/src/pairing.rs`) via QR or pasted text — out of band, no network round trip required for pairing itself:

| Field | Type |
|---|---|
| `identity_key` | `[u8; 32]` — Ed25519 verifying key |
| `curve25519_key` | `[u8; 32]` — vodozemac Olm account key |
| `transport_key` | `[u8; 32]` — this device's **public** iroh endpoint id (not the private key that seeds it) |
| `one_time_key` | `[u8; 32]` — a fresh vodozemac one-time key |
| `nonce` | `[u8; 32]` — fresh random bytes |
| `server_addr` | `Option<Vec<u8>>` — bincode-serialized `EndpointAddr`, `None` if Local-only |

Both sides derive the same `pair_secret` independently, order-independent: sort the two nonces lexicographically (smaller first), concatenate (`first || second`, 64 bytes), then `pm_proto::derive(&combined, "pairing", 0, 32)`. Neither side needs to know who scanned/pasted first.

## 5. Delivery paths

Two peer-to-peer wire protocols exist, both over iroh (QUIC/TLS 1.3), distinguished by ALPN:

### Server mailbox — `NODE_ALPN = b"pm/node/1"`

One bincode-encoded `NodeRequest` read to end-of-stream, one bincode-encoded `NodeResponse` written back, per QUIC connection (`crates/pm-node/src/handler.rs`). Message cap: `MAX_MESSAGE_SIZE = 1 MiB`.

```rust
enum NodeRequest {
    RegisterSlot { mailbox_key: [u8; 32], slot_hash: [u8; 32] },
    Write { auth: [u8; 32], blob: Vec<u8> },
    Fetch { mailbox_key: [u8; 32] },
    Ack { mailbox_key: [u8; 32], ids: Vec<u64> },
    PutBackup { mailbox_key: [u8; 32], blob: Vec<u8> },
    GetBackup { mailbox_key: [u8; 32] },
    ScheduleRetry { mailbox_key: [u8; 32], msg_id: [u8; 16], recipient_transport_key: [u8; 32], envelope: Vec<u8> },
    PollFailedDeliveries { mailbox_key: [u8; 32] },
}

enum NodeResponse {
    Ok,
    Blobs(Vec<StoredBlob>),           // StoredBlob { id: u64, blob: Vec<u8>, delivered: bool }
    Backup(Option<Vec<u8>>),
    FailedDeliveries(Vec<[u8; 16]>),
    Error(String),
}
```

Every variant except `Write` requires `mailbox_key` to match the node's configured owner. `Write` has no owner check by design — the caller is a sender, not the owner — and is instead gated by a one-time write-authorization proof: `slot_hash = SHA256(auth)` is registered in advance (`RegisterSlot`), and a `Write` is accepted only if `SHA256(its auth)` matches a still-unconsumed registered slot, which is then consumed. Each `auth` value is itself `pm_proto::derive(pair_secret, "auth", n, 32)` for a monotonically increasing `n` — so a batch of future write slots can be pre-registered without either side needing to renegotiate per message. `Fetch` returns everything, delivered or not; `Ack` marks delivered but never deletes (`docs/PRD.md` §5/§8 — retained to support planned functionality). `ScheduleRetry`/`PollFailedDeliveries` back the sender's-own-Server retry-delivery path (`docs/PRD.md` Flow 3's third case).

### Direct (Local-to-local) — `DIRECT_ALPN = b"pm/direct/1"`

Simpler: one padded `Envelope` in, one `DirectAck` out, per connection — no request enum wrapping it.

```rust
enum DirectAck { Ok, Error(String) }
```

The receiving side tries to decrypt the incoming envelope against every known contact (existing session pickle if one exists, otherwise a fresh `accept_incoming` attempt using that contact's identity); `Error("could not attribute this message to any known contact")` if none matches. See `docs/PRD.md` §8 for when each delivery path is chosen (Server preferred, Local as automatic fallback).

## 6. Message plaintext

What's actually inside an `Envelope`'s Olm-encrypted `ciphertext` — `PlaintextPayload` (`crates/pm-core/src/message.rs`), bincode-serialized before encryption, never transmitted as raw chat text:

```rust
enum PlaintextPayload {
    Chat(Vec<u8>),
    Receipt { msg_id: [u8; 16] },
    MailboxPointerUpdate { server_addr: Option<Vec<u8>>, updated_at: u64, signature: Vec<u8> },
}
```

`Chat` is a real message; `Receipt` acknowledges one back to its sender (flips `Sent` → `Delivered`, never becomes a visible message itself); `MailboxPointerUpdate` is covered next.

## 7. Signed mailbox-pointer updates

Per `docs/PRD.md` §8: any change to which mailbox represents a user must be signed and verified independently of whatever Olm session happens to carry it, so a compromised or malicious *session* can't redirect a contact's outgoing messages to an attacker-controlled mailbox.

The signed bytes are `bincode::serialize(&(server_addr, updated_at))` — the tuple in that exact order, signature itself never included. Signed with the sender's Ed25519 `signing_key` (§3); verified with `verify_strict` (the non-malleable variant — deliberate, since this gates a security-relevant action) against the sender's known `identity_key`.

Replay/rollback protection is a plain monotonic check, not a nonce or sequence number: an update is accepted only if the signature verifies **and** `updated_at` is strictly greater than the last `updated_at` this receiver has already accepted from that sender. An update with a timestamp equal to or older than the last accepted one is rejected outright, even with a fully valid signature.

## 8. Backup encryption

Manual backup export (`docs/PRD.md` §5, "Recovery on a new device") assembles a `BackupBundle` — account pickle, every contact's identity/session/pending-OTK state, that contact's full message history, and this device's own server address if any — bincode-serializes it, then encrypts with **XChaCha20-Poly1305** keyed by `backup_key` (§3). A fresh random 24-byte nonce is generated per encryption and prepended directly to the ciphertext (`nonce || ciphertext`, no separate length prefix needed since the nonce is fixed-size). The same encrypted blob is what a Server mailbox stores verbatim via `PutBackup`/`GetBackup` (§5) — the node never sees the plaintext bundle, only this ciphertext.

## 9. Local storage encryption

The on-device SQLite database is SQLCipher-encrypted, keyed directly by `mailbox_key` (§3) via SQLCipher's raw-key hex-literal `PRAGMA key = "x'<hex>'"` syntax (not `rusqlite`'s own `pragma_update`, which rejects a raw byte value for this specific pragma).

## 10. Versioning and compatibility

**What exists:** `Envelope.version` is a real wire field, currently always `1`. Each ALPN string itself encodes a literal version suffix (`.../1`) at the QUIC handshake level — a peer offering a mismatched ALPN simply fails to negotiate a connection at all, which is a coarse, binary form of version gating.

**What doesn't exist, stated plainly:** nothing reads or branches on `Envelope.version` after deserialization — it's written but not yet enforced. There is no version field or capability negotiation anywhere in `NodeRequest`/`NodeResponse`/`DirectAck`/`PlaintextPayload`/`PairingPayload`/`BackupBundle`. Compatibility for all of these today is entirely implicit, resting on Rust's struct/enum layout plus bincode's positional (non-self-describing) encoding — bincode has no schema-evolution tolerance, so adding, removing, or reordering a field or enum variant changes the wire encoding incompatibly for every peer at once, with no negotiation step to detect or gracefully reject a mismatch. A genuine mismatch either surfaces as a clean deserialize error (handled today as `NodeResponse::Error`/a dropped `PlaintextPayload`) or, in the worst case, would silently misparse if two incompatible layouts happened to overlap byte-for-byte. This is an accepted, unaddressed gap for the current MVP, not a deliberate compatibility design — `docs/PRD.md`'s own "Open Items" (solo-maintainer project, reproducible builds and this spec as the stated mitigation) is the closest existing acknowledgment of it. Introducing real negotiation (a version handshake, or a self-describing wire format) is future work, not yet started.
