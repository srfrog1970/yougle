//! In-memory mailbox storage. Per `ARCHIT_1.MD` §7's M2 scope ("Node v0...
//! in-memory store"), this does not persist across restarts — that's a
//! later milestone.

use std::collections::HashSet;
use std::sync::Mutex;

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

#[derive(Debug)]
struct Inner {
    registered_slot_hashes: HashSet<[u8; 32]>,
    blobs: Vec<StoredBlob>,
    next_id: u64,
}

/// A single tenant's mailbox: one owner (identified out of band by whatever
/// `mailbox_key` the node is configured to accept — see `handler.rs`), an
/// open set of registered write-authorization hashes, and the blobs
/// deposited against them.
#[derive(Debug)]
pub struct MailboxStore {
    inner: Mutex<Inner>,
}

impl Default for MailboxStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MailboxStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                registered_slot_hashes: HashSet::new(),
                blobs: Vec::new(),
                next_id: 0,
            }),
        }
    }

    pub fn register_slot(&self, slot_hash: [u8; 32]) {
        self.inner
            .lock()
            .unwrap()
            .registered_slot_hashes
            .insert(slot_hash);
    }

    /// Verifies `SHA256(auth)` matches a registered slot, consuming it, and
    /// stores `blob` if so.
    pub fn write(&self, auth: [u8; 32], blob: Vec<u8>) -> Result<u64, WriteError> {
        let slot_hash: [u8; 32] = Sha256::digest(auth).into();
        let mut inner = self.inner.lock().unwrap();
        if !inner.registered_slot_hashes.remove(&slot_hash) {
            return Err(WriteError::NoMatchingSlot);
        }
        let id = inner.next_id;
        inner.next_id += 1;
        inner.blobs.push(StoredBlob {
            id,
            blob,
            delivered: false,
        });
        Ok(id)
    }

    /// Everything currently stored, delivered or not.
    pub fn fetch_all(&self) -> Vec<StoredBlob> {
        self.inner.lock().unwrap().blobs.clone()
    }

    /// Marks the given ids delivered. Unknown ids are silently ignored
    /// (idempotent acking of something already gone, e.g. by a future
    /// retention sweep, shouldn't be an error).
    pub fn ack(&self, ids: &[u64]) {
        let mut inner = self.inner.lock().unwrap();
        for blob in inner.blobs.iter_mut() {
            if ids.contains(&blob.id) {
                blob.delivered = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(auth: [u8; 32]) -> [u8; 32] {
        Sha256::digest(auth).into()
    }

    #[test]
    fn write_without_a_registered_slot_is_rejected() {
        let store = MailboxStore::new();
        let result = store.write([1u8; 32], b"hello".to_vec());
        assert_eq!(result, Err(WriteError::NoMatchingSlot));
        assert!(store.fetch_all().is_empty());
    }

    #[test]
    fn write_with_a_registered_slot_succeeds_and_stores_the_blob() {
        let store = MailboxStore::new();
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
        let store = MailboxStore::new();
        let auth = [7u8; 32];
        store.register_slot(hash(auth));

        assert!(store.write(auth, b"first".to_vec()).is_ok());
        let second = store.write(auth, b"second".to_vec());
        assert_eq!(second, Err(WriteError::NoMatchingSlot));
        assert_eq!(store.fetch_all().len(), 1);
    }

    #[test]
    fn ack_marks_delivered_without_removing_the_blob() {
        let store = MailboxStore::new();
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
        let store = MailboxStore::new();
        store.ack(&[999]); // must not panic
    }
}
