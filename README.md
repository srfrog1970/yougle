# Yougle

A privacy-focused, decentralized text messenger. No accounts, no phone numbers, no push notifications, no shared third-party infrastructure.

Every user gets an automatic, client-side **Local** mailbox for direct, live delivery. Anyone can optionally run their own self-hosted **Server** mailbox for durable, asynchronous delivery — always owned by the person running it, never a shared community operator.

**Just want to install the app and use it?** See [`QUICKSTART.md`](QUICKSTART.md) — download, sideload, pair, send, and optionally connect your own node, no development environment needed.

Status: **M0–M7 complete, plus signed mailbox-pointer updates and a verified self-hosting path.** The product requirements are locked (see [`docs/PRD.md`](docs/PRD.md)). `pm-proto` (envelopes, padding, chain derivation, node wire protocol), `pm-crypto` (seed handling, identity derivation, Olm sessions, backup encryption), `pm-store` (SQLCipher-encrypted persistence, Lamport ordering), `pm-transport` (iroh client), `pm-node` (the Server mailbox binary, SQLCipher-encrypted persistent storage opt-in via `PM_NODE_DATA_DIR`, plus M7's outbound retry-delivery sweep), and `pm-core` (the client tying all of the above into pairing/`send`/`sync`/backup/restore, Local-to-local direct delivery and real "delivered" receipts, sender's-own-Server scheduled retry for an unreachable recipient, and signed/verified mailbox-pointer updates per `docs/PRD.md` §8) are implemented and tested. `pm-ffi` (the uniffi interface exposing `pm-core`) mirrors that full surface for the app. `app/` is a real React Native app — onboarding, conversation list, chat (with live sent/delivered/failed status), QR/paste-code pairing, mailbox management, a "set up your own node" key-export screen, recovery-phrase view, and backup export/import, all wired to the live Rust core, not mocks — verified by actually running it on two simultaneous headless Android emulators pairing and exchanging live messages (see [`app/README.md`](app/README.md)). `deploy/` (Docker and systemd, for self-hosting `pm-node`) has been verified end to end on real hardware, not just written — see [`deploy/README.md`](deploy/README.md)'s Status line for exactly what was and wasn't exercised. A properly-signed release APK (not the debug keystore) builds cleanly and was smoke-tested on-device with no Metro/dev server running at all — see [`QUICKSTART.md`](QUICKSTART.md). iOS remains impossible outside macOS/Xcode.

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
  PROTOCOL.md     wire format spec
  THREAT-MODEL.md what is and is not claimed
  adr/            architectural decision records
deploy/            Dockerfile, compose, systemd unit for pm-node
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Dependency selection prefers MIT-licensed libraries where a suitable option exists; already-chosen dependencies under other permissive licenses (e.g., vodozemac, Apache-2.0) are not affected.
