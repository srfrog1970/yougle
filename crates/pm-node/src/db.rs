//! Shared SQLCipher connection open/key/migrate mechanism for `pm-node`'s
//! own storage. A small, independent copy of `pm-store`'s own open+migrate
//! pattern (see `crates/pm-store/src/lib.rs`/`migrations.rs`), not a
//! dependency on `pm-store` itself — `pm-store`'s migration numbering is
//! `pm-core`'s own chat/contacts/sessions history, and `pm-node`'s
//! server-side tables (registered slots, blobs, retry queue) have nothing
//! to do with it; a phone app's local DB and a self-hosted server's mailbox
//! DB are operationally independent products that in the common case never
//! even run on the same machine.

use std::path::Path;
use std::sync::{Arc, Mutex};

use hkdf::Hkdf;
use rusqlite::Connection;
use sha2::Sha256;
use thiserror::Error;

use crate::migrations;

/// Domain-separation label for `pm-node`'s at-rest storage key,
/// HKDF-expanded from `PM_NODE_MAILBOX_KEY` — kept distinct from that
/// value's *other* use (the wire-facing owner-auth bearer credential
/// `handler.rs` compares incoming requests against) purely as hygiene, not
/// a load-bearing cryptographic boundary: `storage_key` is a deterministic,
/// unkeyed function of a value already resident in this process either
/// way, so this just avoids literally reusing the network-facing
/// credential bytes as a disk-encryption key.
const STORAGE_KEY_LABEL: &[u8] = b"pm-node-storage-key-v1";

#[derive(Debug, Error)]
pub(crate) enum DbError {
    #[error("failed to open or migrate storage: {0}")]
    Open(#[from] rusqlite::Error),
}

fn hkdf_expand_32(ikm: &[u8], label: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 32];
    hk.expand(label, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}

/// Opens (creating if needed) `pm-node`'s own SQLCipher database: in-memory
/// if `data_dir` is `None` (today's default, ephemeral behavior — still
/// keyed and migrated through the identical code path as the persistent
/// case, so there's exactly one implementation to maintain), on-disk at
/// `<data_dir>/mailbox.sqlite` otherwise.
///
/// A wrong `mailbox_key` against a *fresh* database fails promptly here,
/// since the first migration's `CREATE TABLE` touches real pages. Against
/// an *already-migrated* database, though, SQLCipher's `PRAGMA key` accepts
/// any key unconditionally, and `migrations::run` may see `PRAGMA
/// user_version` already at its max and no-op without ever touching a real
/// page — so a wrong key there returns `Ok` here, and the failure only
/// surfaces on the first real read/write against the resulting store,
/// where it panics (see `store.rs`/`retry_queue.rs`'s "fatal on I/O error"
/// convention). Documented, not fixed — not worth building around for a
/// single-tenant background daemon that a supervisor (systemd/Docker) just
/// restarts on crash anyway.
pub(crate) fn open(
    data_dir: Option<&Path>,
    mailbox_key: [u8; 32],
) -> Result<Arc<Mutex<Connection>>, DbError> {
    let conn = match data_dir {
        Some(dir) => Connection::open(dir.join("mailbox.sqlite"))?,
        None => Connection::open_in_memory()?,
    };
    let storage_key = hkdf_expand_32(&mailbox_key, STORAGE_KEY_LABEL);
    // rusqlite's pragma_update won't accept a Blob value for a PRAGMA
    // statement, so the raw-key hex literal is built and executed
    // directly instead — SQLCipher's own documented raw-key syntax, not a
    // rusqlite API. Same approach as pm-store's `Store::init`.
    conn.execute_batch(&format!(
        "PRAGMA key = \"x'{}'\";",
        hex::encode(storage_key)
    ))?;
    migrations::run(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// An in-memory store keyed with a fixed test key — used by every unit
/// test in `store.rs`/`retry_queue.rs`/`handler.rs` that doesn't itself
/// care about persistence, matching `spawn(..., data_dir: None)`'s exact
/// code path.
#[cfg(test)]
pub(crate) fn open_for_test() -> Arc<Mutex<Connection>> {
    open(None, [0u8; 32]).expect("in-memory open with a fixed test key cannot fail")
}
