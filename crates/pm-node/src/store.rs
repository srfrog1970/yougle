//! `pm-node`'s mailbox storage: registered write-authorization slots, the
//! stored blobs deposited against them, and the owner's backup blob.
//! SQLCipher-encrypted at rest (see `db.rs`) — persists across restarts
//! when `spawn` is given a `data_dir`, in-memory otherwise.

use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub id: u64,
    pub blob: Vec<u8>,
    pub delivered: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// No registered slot's hash matches the presented `auth` — either
    /// nobody registered it, or it's already been consumed by a prior
    /// write.
    NoMatchingSlot,
}

/// A single tenant's mailbox: one owner (identified out of band by whatever
/// `mailbox_key` the node is configured to accept — see `handler.rs`), an
/// open set of registered write-authorization hashes, and the blobs
/// deposited against them. Shares its connection with [`crate::RetryQueue`]
/// (both are handed the same `Arc<Mutex<Connection>>` by `spawn`).
#[derive(Debug)]
pub struct MailboxStore {
    conn: Arc<Mutex<Connection>>,
}

impl MailboxStore {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn register_slot(&self, slot_hash: [u8; 32]) {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT OR IGNORE INTO registered_slots (slot_hash) VALUES (?1)",
                params![slot_hash.as_slice()],
            )
            .expect("register_slot: storage I/O failure after a successful open+migration");
    }

    /// Verifies `SHA256(auth)` matches a registered slot, consuming it, and
    /// stores `blob` if so. The consume-then-store pair runs in one
    /// transaction so a crash between the two can't leave the slot
    /// consumed with no blob actually stored.
    pub fn write(&self, auth: [u8; 32], blob: Vec<u8>) -> Result<u64, WriteError> {
        let slot_hash: [u8; 32] = Sha256::digest(auth).into();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .expect("write: storage I/O failure after a successful open+migration");

        let deleted = tx
            .execute(
                "DELETE FROM registered_slots WHERE slot_hash = ?1",
                params![slot_hash.as_slice()],
            )
            .expect("write: storage I/O failure after a successful open+migration");
        if deleted == 0 {
            return Err(WriteError::NoMatchingSlot);
        }

        tx.execute(
            "INSERT INTO blobs (blob, delivered) VALUES (?1, 0)",
            params![blob],
        )
        .expect("write: storage I/O failure after a successful open+migration");
        let id = tx.last_insert_rowid();
        tx.commit()
            .expect("write: storage I/O failure after a successful open+migration");
        Ok(id as u64)
    }

    /// Everything currently stored, delivered or not.
    pub fn fetch_all(&self) -> Vec<StoredBlob> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, blob, delivered FROM blobs ORDER BY id")
            .expect("fetch_all: storage I/O failure after a successful open+migration");
        stmt.query_map([], |row| {
            Ok(StoredBlob {
                id: row.get::<_, i64>(0)? as u64,
                blob: row.get(1)?,
                delivered: row.get::<_, i64>(2)? != 0,
            })
        })
        .expect("fetch_all: storage I/O failure after a successful open+migration")
        .collect::<rusqlite::Result<_>>()
        .expect("fetch_all: storage I/O failure after a successful open+migration")
    }

    /// Marks the given ids delivered. Unknown ids are silently ignored
    /// (idempotent acking of something already gone, e.g. by a future
    /// retention sweep, shouldn't be an error).
    pub fn ack(&self, ids: &[u64]) {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .expect("ack: storage I/O failure after a successful open+migration");
        for &id in ids {
            tx.execute(
                "UPDATE blobs SET delivered = 1 WHERE id = ?1",
                params![id as i64],
            )
            .expect("ack: storage I/O failure after a successful open+migration");
        }
        tx.commit()
            .expect("ack: storage I/O failure after a successful open+migration");
    }

    /// Replaces the current backup blob wholesale.
    pub fn put_backup(&self, blob: Vec<u8>) {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO backup (id, blob) VALUES (0, ?1) \
                 ON CONFLICT(id) DO UPDATE SET blob = excluded.blob",
                params![blob],
            )
            .expect("put_backup: storage I/O failure after a successful open+migration");
    }

    /// The current backup blob, if one has ever been stored.
    pub fn get_backup(&self) -> Option<Vec<u8>> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT blob FROM backup WHERE id = 0", [], |row| row.get(0))
            .optional()
            .expect("get_backup: storage I/O failure after a successful open+migration")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn hash(auth: [u8; 32]) -> [u8; 32] {
        Sha256::digest(auth).into()
    }

    fn store() -> MailboxStore {
        MailboxStore::new(db::open_for_test())
    }

    #[test]
    fn write_without_a_registered_slot_is_rejected() {
        let store = store();
        let result = store.write([1u8; 32], b"hello".to_vec());
        assert_eq!(result, Err(WriteError::NoMatchingSlot));
        assert!(store.fetch_all().is_empty());
    }

    #[test]
    fn write_with_a_registered_slot_succeeds_and_stores_the_blob() {
        let store = store();
        let auth = [7u8; 32];
        store.register_slot(hash(auth));

        let id = store.write(auth, b"hello".to_vec()).unwrap();

        let blobs = store.fetch_all();
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].id, id);
        assert_eq!(blobs[0].blob, b"hello");
        assert!(!blobs[0].delivered);
    }

    #[test]
    fn a_slot_can_only_be_used_once() {
        let store = store();
        let auth = [7u8; 32];
        store.register_slot(hash(auth));

        assert!(store.write(auth, b"first".to_vec()).is_ok());
        let second = store.write(auth, b"second".to_vec());
        assert_eq!(second, Err(WriteError::NoMatchingSlot));
        assert_eq!(store.fetch_all().len(), 1);
    }

    #[test]
    fn ack_marks_delivered_without_removing_the_blob() {
        let store = store();
        let auth = [7u8; 32];
        store.register_slot(hash(auth));
        let id = store.write(auth, b"hello".to_vec()).unwrap();

        store.ack(&[id]);

        let blobs = store.fetch_all();
        assert_eq!(blobs.len(), 1, "acked blobs are retained, not deleted");
        assert!(blobs[0].delivered);
    }

    #[test]
    fn acking_an_unknown_id_is_a_harmless_no_op() {
        let store = store();
        store.ack(&[999]); // must not panic
    }

    #[test]
    fn backup_starts_empty_then_roundtrips_and_replaces() {
        let store = store();
        assert_eq!(store.get_backup(), None);

        store.put_backup(b"first backup".to_vec());
        assert_eq!(store.get_backup(), Some(b"first backup".to_vec()));

        store.put_backup(b"second backup".to_vec());
        assert_eq!(
            store.get_backup(),
            Some(b"second backup".to_vec()),
            "put_backup replaces wholesale, doesn't append"
        );
    }
}
