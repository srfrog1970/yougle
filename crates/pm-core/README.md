# pm-core

State machine, sync engine, and the public API surface consumed by the app through `pm-ffi`. Ties `pm-proto`, `pm-crypto`, `pm-store`, and `pm-transport` together into one `Client`.

Status: M3 complete, scoped as below.

- `client::Client` — `open` (creates or resumes a local identity+store, loading/creating the vodozemac account), `add_contact` (pairing stand-in), `send`/`sync` (the sync engine), `push_backup`/`restore`.
- `backup` — assembles/restores the backup bundle contents (contacts, pairing state, session pickles, message history, this device's own account pickle) that `pm_crypto::backup` encrypts as opaque bytes.

**Scope, and why:**

- **Only Server-mailbox delivery is wired up.** Direct Local (phone-to-phone) delivery already works at the `pm-transport`/`pm-crypto` level (proven in the M1/M2 tests), but integrating it into `Client` needs `Client` to also run its own accept loop while foregrounded — deferred to M4/M5, when the app actually needs both modes.
- **Restoring on a new device means re-supplying the Server's address directly, not an automatic DHT lookup.** `ARCHIT_1.MD` itself flags DHT-based pointer publishing as the one genuinely open mechanism question in the original design ("may require using mainline DHT directly... resolve during M2"). Rather than guess at an unverified integration, this build extends the original architecture's own `PUT_BACKUP`/`GET_BACKUP` node calls (now in `pm-node`) and has the caller supply the Server address on restore. Real, honest v0 — not a resolution of the open item.
- **The Olm session-establishment bug this milestone surfaced is worth knowing about**: pairing gives both sides a one-time key from the other, and it's tempting to have both sides eagerly call `create_outbound_session` immediately. Don't — that produces two independent, mismatched Olm sessions instead of one shared one. Exactly one side's session gets established, lazily, on whichever side sends first (`Client::send`, using a one-time key held in `pending_otk` until then).

`tests/m3_exit_criteria.rs` covers M3's exit criteria end to end: two clients converse normally, one pushes a backup to their own Server, their device (and local store) is dropped entirely, a fresh `Client::restore` on a brand-new store recovers the same identity plus full contact/message history, and — not just displaying old history, but actually continuing the live conversation — sends and receives a new message afterward.

## Running the tests locally

Requires `libssl-dev` and `pkg-config` (for `pm-store`'s bundled SQLCipher build — see its README).

```
cargo test -p pm-core
cargo clippy -p pm-core --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
