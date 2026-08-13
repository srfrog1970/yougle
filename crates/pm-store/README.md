# pm-store

SQLCipher-encrypted local storage: schema, migrations, and Lamport-clock message ordering.

Status: M1 and M3 complete.

- Schema, across four migrations: `contacts`, `sessions` (pickled Olm state per contact), `messages`, `local_clock` (0001); this device's own `account` pickle (0002); per-contact `pair_secret` and `next_write_n` for deriving write-auth values, plus a `last_synced_blob_id` watermark (0003); a `pending_otk` per contact, holding the one-time key received at pairing until consumed by the first outbound send (0004).
- `migrations` — a minimal linear runner tracked via `PRAGMA user_version`.
- `lamport` — the standard Lamport clock (`tick`, `observe`), persisted so it survives restarts.
- The crate itself is deliberately identity/crypto-agnostic — it stores whatever key bytes and plaintext it's given, and only depends on `pm-proto`/`pm-crypto` as dev-dependencies for the milestone integration test. Wiring storage to the crypto/session layer for real is `pm-core`'s job.

Encryption is real SQLCipher (`rusqlite`'s `bundled-sqlcipher` feature, built against system OpenSSL — needs `libssl-dev` + `pkg-config` to build), not a plaintext fallback: `opening_with_the_wrong_key_fails_to_read_existing_data` verifies a wrong key produces a genuine SQLCipher HMAC failure, not silent garbage or success.

`tests/m1_exit_criteria.rs` covers the full M1 milestone exit criteria end to end: two identities (`pm-crypto`) exchange Olm-encrypted messages framed as `pm-proto` envelopes, persist them (and their session state) to separate encrypted stores, survive a simulated app restart, and resume the conversation.

## Running the tests locally

Requires `libssl-dev` and `pkg-config` (Debian/Ubuntu: `sudo apt-get install -y libssl-dev pkg-config`) for the bundled SQLCipher build.

```
cargo test -p pm-store
cargo clippy -p pm-store --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
