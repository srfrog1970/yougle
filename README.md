# Yougle

A privacy-focused, decentralized text messenger. No accounts, no phone numbers, no push notifications, no shared third-party infrastructure.

Every user gets an automatic, client-side **Local** mailbox for direct, live delivery. Anyone can optionally run their own self-hosted **Server** mailbox for durable, asynchronous delivery — always owned by the person running it, never a shared community operator.

Status: **M0–M5 complete.** The product requirements are locked (see [`docs/PRD.md`](docs/PRD.md)). `pm-proto` (envelopes, padding, chain derivation, node wire protocol), `pm-crypto` (seed handling, identity derivation, Olm sessions, backup encryption), `pm-store` (SQLCipher-encrypted persistence, Lamport ordering), `pm-transport` (iroh client), `pm-node` (the Server mailbox binary, v0/in-memory), and `pm-core` (the client tying all of the above into pairing/`send`/`sync`/backup/restore) are implemented and tested. `pm-ffi` (the uniffi interface exposing `pm-core`) mirrors that full surface for the app. `app/` is a real React Native app — onboarding, conversation list, chat, QR/paste-code pairing, mailbox management, recovery-phrase view, and backup export/import, all wired to the live Rust core, not mocks — verified by actually running it on a headless Android emulator (see [`app/README.md`](app/README.md) for the build command, the WSL2 kernel workaround it needs, and the emulator setup). Local-to-local live delivery and a true "delivered" receipt are **M6**, not yet built — see `pm-core`'s module docs. iOS remains impossible outside macOS/Xcode.

Building requires `libssl-dev` and `pkg-config` (Debian/Ubuntu: `sudo apt-get install -y libssl-dev pkg-config`) for the bundled SQLCipher build in `pm-store`.

```
cargo test    # run all workspace tests
```

## Repository layout

```
crates/
  pm-proto/       envelopes, tag derivation, versioning
  pm-crypto/      vodozemac wrapper, HKDF, BIP39 seed handling
  pm-store/       SQLCipher schema, migrations, Lamport ordering
  pm-transport/   iroh endpoint, mailbox client, pointer records
  pm-core/        state machine, sync engine, public API surface
  pm-ffi/         uniffi interface definitions
  pm-node/        Server mailbox binary (self-hosted, single-tenant)
app/               React Native UI (TypeScript)
docs/
  PRD.md          product requirements
  PROTOCOL.md     wire spec (TBD)
  THREAT-MODEL.md what is and is not claimed (TBD)
  adr/            architectural decision records
deploy/            Dockerfile, compose, systemd unit for pm-node
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Dependency selection prefers MIT-licensed libraries where a suitable option exists; already-chosen dependencies under other permissive licenses (e.g., vodozemac, Apache-2.0) are not affected.
