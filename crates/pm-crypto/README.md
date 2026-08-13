# pm-crypto

vodozemac (Olm double ratchet) wrapper, HKDF key derivation, BIP39 seed handling, identity derivation, and backup encryption.

Status: M1 and M3 crypto pieces complete.

- `seed` — BIP39 24-word recovery phrase generation/parsing, using the mnemonic's 32-byte entropy (not the 64-byte BIP32-style `to_seed` value) as the seed for everything below.
- `identity` — deterministic derivation of the identity signing key (IK, Ed25519) and three raw key-material values (backup, mailbox, backup-location) from the seed via HKDF-SHA256, with a pinned test vector. Always recoverable from the seed phrase alone, per `docs/PRD.md`.
- `session` — wraps vodozemac's `Account`/`Session` (Olm double ratchet) for encrypt/decrypt, adapting message framing to the `(olm_type, ciphertext)` shape `pm_proto::Envelope` stores, plus `pickle`/`from_pickle` so account and session state survive a restart. These session keys are randomly generated per account and are *not* derived from the seed — intentional, since sessions re-key on recovery rather than restoring old ratchet state.
- `backup` — encrypts/decrypts an opaque backup blob under a user's `backup_key` (XChaCha20-Poly1305, fresh nonce per encryption). What goes *inside* the blob is `pm-core`'s job.

Two in-process accounts exchanging encrypted messages in both directions (M1's exit criteria) is covered by `session::tests::two_accounts_exchange_messages_in_both_directions`.

## Running the tests locally

```
cargo test -p pm-crypto
cargo clippy -p pm-crypto --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
