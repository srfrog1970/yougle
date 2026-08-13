//! `pm-store`: SQLCipher-encrypted local storage. Deliberately
//! identity/crypto-agnostic — it stores whatever key bytes and message
//! plaintext it's given, and doesn't depend on `pm-crypto` or `pm-proto`
//! (those appear only as dev-dependencies, for the milestone integration
//! test in `tests/`). Integrating storage with the crypto/session layer is
//! `pm-core`'s job.

pub mod error;
mod lamport;
mod migrations;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use error::Result;
pub use error::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Outgoing => "outgoing",
            Direction::Incoming => "incoming",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "outgoing" => Direction::Outgoing,
            _ => Direction::Incoming,
        }
    }
}

/// Delivery status for an outgoing message (see migration 0006). Never set
/// for incoming messages — `NewMessage::status`/`StoredMessage::status` are
/// `None` there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MessageStatus {
    Sent,
    Delivered,
    Failed,
}

impl MessageStatus {
    fn as_str(self) -> &'static str {
        match self {
            MessageStatus::Sent => "sent",
            MessageStatus::Delivered => "delivered",
            MessageStatus::Failed => "failed",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "sent" => Some(MessageStatus::Sent),
            "delivered" => Some(MessageStatus::Delivered),
            "failed" => Some(MessageStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRecord {
    pub id: i64,
    pub identity_key: [u8; 32],
    pub curve25519_key: [u8; 32],
    pub display_name: Option<String>,
    pub server_addr: Option<Vec<u8>>,
    pub pair_secret: Option<[u8; 32]>,
    pub next_write_n: u64,
    /// The one-time key received from this contact at pairing time, not yet
    /// consumed by a first outbound message. See migration 0004 for why
    /// this exists rather than an eagerly-established session.
    pub pending_otk: Option<[u8; 32]>,
    /// This contact's iroh transport public key, shared at pairing — lets
    /// this device dial them directly for Local delivery (see M6).
    pub transport_key: Option<[u8; 32]>,
}

pub struct NewMessage<'a> {
    pub msg_id: [u8; 16],
    pub direction: Direction,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: &'a [u8],
    pub status: Option<MessageStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredMessage {
    pub msg_id: [u8; 16],
    pub direction: Direction,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: Vec<u8>,
    pub status: Option<MessageStatus>,
}

/// A single user's encrypted local database: contacts, session state, and
/// message history.
///
/// The connection is `Mutex`-wrapped so `Store` (and everything built on
/// it, up through `pm-core`'s `Client`) is `Sync` — `rusqlite::Connection`
/// itself isn't (it uses `RefCell` internally for statement caching), which
/// otherwise blocks any async method taking `&self` from producing a `Send`
/// future. That doesn't matter when a future is only ever directly
/// `.await`ed (as in this crate's own tests), but it does the moment
/// something spawns it onto a multi-threaded executor — which is exactly
/// what `pm-ffi`'s uniffi async bridging does.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) a SQLCipher database at `path`, keyed by
    /// `key`. The key is applied via `PRAGMA key` (as a raw-key hex literal,
    /// SQLCipher's own convention — see [`Store::init`]) before any other
    /// statement runs.
    pub fn open(path: &Path, key: &[u8]) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn, key)
    }

    /// An in-memory store, still SQLCipher-keyed. Useful for tests; not
    /// meaningfully "encrypted" since it never touches disk, but exercises
    /// the same code path as [`Store::open`].
    pub fn open_in_memory(key: &[u8]) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn, key)
    }

    fn init(conn: Connection, key: &[u8]) -> Result<Self> {
        // rusqlite's pragma_update won't accept a Blob value for a PRAGMA
        // statement (confirmed empirically: it errors with ApiMisuse), so
        // the raw-key hex literal is built and executed directly instead.
        // This is SQLCipher's own documented raw-key syntax, not a rusqlite
        // API — see https://www.zetetic.net/sqlcipher/sqlcipher-api/#key.
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(key)))?;
        migrations::run(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Inserts a contact, or updates its keys/name if the identity key is
    /// already known. Returns the contact's local row id.
    pub fn upsert_contact(
        &self,
        identity_key: &[u8; 32],
        curve25519_key: &[u8; 32],
        display_name: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO contacts (identity_key, curve25519_key, display_name, created_at)
             VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT (identity_key) DO UPDATE SET
                curve25519_key = excluded.curve25519_key,
                display_name = excluded.display_name",
            params![
                identity_key.as_slice(),
                curve25519_key.as_slice(),
                display_name
            ],
        )?;
        conn.query_row(
            "SELECT id FROM contacts WHERE identity_key = ?1",
            [identity_key.as_slice()],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    /// Saves (or replaces) the pickled Olm session state for a contact.
    pub fn save_session_pickle(&self, contact_id: i64, pickle: &[u8]) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO sessions (contact_id, pickle, updated_at)
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT (contact_id) DO UPDATE SET
                pickle = excluded.pickle,
                updated_at = excluded.updated_at",
            params![contact_id, pickle],
        )?;
        Ok(())
    }

    pub fn load_session_pickle(&self, contact_id: i64) -> Result<Option<Vec<u8>>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT pickle FROM sessions WHERE contact_id = ?1",
                [contact_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_message(&self, contact_id: i64, msg: NewMessage) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO messages (contact_id, msg_id, direction, lamport, sent_at, plaintext, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())",
            params![
                contact_id,
                msg.msg_id.as_slice(),
                msg.direction.as_str(),
                msg.lamport as i64,
                msg.sent_at as i64,
                msg.plaintext,
                msg.status.map(MessageStatus::as_str),
            ],
        )?;
        Ok(())
    }

    /// Updates an outgoing message's delivery status, looked up by its
    /// `msg_id` (unique per contact — see migration 0001's UNIQUE
    /// constraint). A no-op if no message with that id exists for this
    /// contact (e.g. a receipt arriving for a message this device never
    /// actually sent — ignored, not an error).
    pub fn set_message_status(
        &self,
        contact_id: i64,
        msg_id: &[u8; 16],
        status: MessageStatus,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE messages SET status = ?1 WHERE contact_id = ?2 AND msg_id = ?3",
            params![status.as_str(), contact_id, msg_id.as_slice()],
        )?;
        Ok(())
    }

    /// Same as `set_message_status`, but looked up by `msg_id` alone — for
    /// M7's `PollFailedDeliveries`, which reports back which of the
    /// owner's own queued sends gave up without knowing (or needing to
    /// know) which contact it was to; `pm-node` has no notion of contacts
    /// at all. `msg_id` is 128 bits of randomness (see `deliver`'s
    /// generation in `pm-core`), so matching on it alone is safe in
    /// practice even though the schema's own uniqueness constraint is
    /// scoped per-contact.
    pub fn set_message_status_by_msg_id(
        &self,
        msg_id: &[u8; 16],
        status: MessageStatus,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE messages SET status = ?1 WHERE msg_id = ?2",
            params![status.as_str(), msg_id.as_slice()],
        )?;
        Ok(())
    }

    /// All messages with a contact, oldest first by Lamport order.
    pub fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT msg_id, direction, lamport, sent_at, plaintext, status
             FROM messages WHERE contact_id = ?1 ORDER BY lamport ASC",
        )?;
        let rows = stmt.query_map([contact_id], |row| {
            let msg_id: Vec<u8> = row.get(0)?;
            let direction: String = row.get(1)?;
            let lamport: i64 = row.get(2)?;
            let sent_at: i64 = row.get(3)?;
            let plaintext: Vec<u8> = row.get(4)?;
            let status: Option<String> = row.get(5)?;
            Ok((msg_id, direction, lamport, sent_at, plaintext, status))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (msg_id, direction, lamport, sent_at, plaintext, status) = row?;
            let msg_id: [u8; 16] = msg_id
                .try_into()
                .map_err(|v: Vec<u8>| StoreError::InvalidMsgIdLength(v.len()))?;
            out.push(StoredMessage {
                msg_id,
                direction: Direction::from_str(&direction),
                lamport: lamport as u64,
                sent_at: sent_at as u64,
                plaintext,
                status: status.and_then(|s| MessageStatus::from_str(&s)),
            });
        }
        Ok(out)
    }

    /// Advances this device's Lamport clock for a local event (e.g., sending
    /// a message) and returns the new value.
    pub fn tick_lamport(&self) -> Result<u64> {
        lamport::tick(&self.conn.lock().unwrap())
    }

    /// Merges an observed remote Lamport value (e.g., from a received
    /// message) into this device's clock and returns the new local value.
    pub fn observe_lamport(&self, remote: u64) -> Result<u64> {
        lamport::observe(&self.conn.lock().unwrap(), remote)
    }

    /// Saves (or replaces) this device's own vodozemac Account pickle.
    pub fn save_account_pickle(&self, pickle: &[u8]) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO account (id, pickle, updated_at) VALUES (0, ?1, unixepoch())
             ON CONFLICT (id) DO UPDATE SET
                pickle = excluded.pickle,
                updated_at = excluded.updated_at",
            params![pickle],
        )?;
        Ok(())
    }

    pub fn load_account_pickle(&self) -> Result<Option<Vec<u8>>> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT pickle FROM account WHERE id = 0", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(Into::into)
    }

    /// Records where a contact can currently be reached — an opaque,
    /// serialized address `pm-core` supplies and interprets (e.g., an iroh
    /// `EndpointAddr`), or `None` to clear it back to unknown/Local-only.
    pub fn set_contact_server_addr(
        &self,
        contact_id: i64,
        server_addr: Option<&[u8]>,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE contacts SET server_addr = ?1 WHERE id = ?2",
            params![server_addr, contact_id],
        )?;
        Ok(())
    }

    pub fn get_contact_server_addr(&self, contact_id: i64) -> Result<Option<Vec<u8>>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT server_addr FROM contacts WHERE id = ?1",
                [contact_id],
                |row| row.get(0),
            )
            .optional()
            .map(|opt| opt.flatten())
            .map_err(Into::into)
    }

    /// Records a contact's iroh transport public key, shared at pairing
    /// time — see `ContactRecord::transport_key`.
    pub fn set_contact_transport_key(
        &self,
        contact_id: i64,
        transport_key: &[u8; 32],
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE contacts SET transport_key = ?1 WHERE id = ?2",
            params![transport_key.as_slice(), contact_id],
        )?;
        Ok(())
    }

    /// Records the shared pairing secret for a contact (stand-in for real
    /// pairing output — see `pm-core`).
    pub fn set_contact_pair_secret(&self, contact_id: i64, pair_secret: &[u8; 32]) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE contacts SET pair_secret = ?1 WHERE id = ?2",
            params![pair_secret.as_slice(), contact_id],
        )?;
        Ok(())
    }

    /// Atomically advances and returns this device's write counter for a
    /// contact — the `n` used to derive the next write-auth value.
    pub fn increment_and_get_next_write_n(&self, contact_id: i64) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE contacts SET next_write_n = next_write_n + 1 WHERE id = ?1",
            [contact_id],
        )?;
        conn.query_row(
            "SELECT next_write_n FROM contacts WHERE id = ?1",
            [contact_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| (n - 1) as u64) // return the value *used* for this write, not the post-increment one
        .map_err(Into::into)
    }

    /// Records the one-time key received from this contact at pairing time,
    /// held until it's consumed to lazily establish the first outbound
    /// session (see migration 0004).
    pub fn set_contact_pending_otk(&self, contact_id: i64, otk: Option<&[u8; 32]>) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "UPDATE contacts SET pending_otk = ?1 WHERE id = ?2",
            params![otk.map(|k| k.as_slice()), contact_id],
        )?;
        Ok(())
    }

    pub fn list_contacts(&self) -> Result<Vec<ContactRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, identity_key, curve25519_key, display_name, server_addr, pair_secret, next_write_n, pending_otk, transport_key
             FROM contacts ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let identity_key: Vec<u8> = row.get(1)?;
            let curve25519_key: Vec<u8> = row.get(2)?;
            let display_name: Option<String> = row.get(3)?;
            let server_addr: Option<Vec<u8>> = row.get(4)?;
            let pair_secret: Option<Vec<u8>> = row.get(5)?;
            let next_write_n: i64 = row.get(6)?;
            let pending_otk: Option<Vec<u8>> = row.get(7)?;
            let transport_key: Option<Vec<u8>> = row.get(8)?;
            Ok((
                id,
                identity_key,
                curve25519_key,
                display_name,
                server_addr,
                pair_secret,
                next_write_n,
                pending_otk,
                transport_key,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (
                id,
                identity_key,
                curve25519_key,
                display_name,
                server_addr,
                pair_secret,
                next_write_n,
                pending_otk,
                transport_key,
            ) = row?;
            out.push(ContactRecord {
                id,
                identity_key: identity_key
                    .try_into()
                    .map_err(|_| StoreError::InvalidMsgIdLength(0))?,
                curve25519_key: curve25519_key
                    .try_into()
                    .map_err(|_| StoreError::InvalidMsgIdLength(0))?,
                display_name,
                server_addr,
                pair_secret: pair_secret.and_then(|v| v.try_into().ok()),
                next_write_n: next_write_n as u64,
                pending_otk: pending_otk.and_then(|v| v.try_into().ok()),
                transport_key: transport_key.and_then(|v| v.try_into().ok()),
            });
        }
        Ok(out)
    }

    pub fn get_contact(&self, contact_id: i64) -> Result<Option<ContactRecord>> {
        Ok(self
            .list_contacts()?
            .into_iter()
            .find(|c| c.id == contact_id))
    }

    /// High-water mark of node-assigned blob ids already processed by
    /// `sync()`.
    pub fn get_last_synced_blob_id(&self) -> Result<u64> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT last_synced_blob_id FROM account WHERE id = 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|opt| opt.unwrap_or(0) as u64)
            .map_err(Into::into)
    }

    pub fn set_last_synced_blob_id(&self, value: u64) -> Result<()> {
        // The account row might not exist yet if no account pickle has been
        // saved; ensure it does, then set the watermark.
        self.conn.lock().unwrap().execute(
            "INSERT INTO account (id, pickle, updated_at, last_synced_blob_id)
             VALUES (0, x'', unixepoch(), ?1)
             ON CONFLICT (id) DO UPDATE SET last_synced_blob_id = excluded.last_synced_blob_id",
            [value as i64],
        )?;
        Ok(())
    }

    /// This device's own Server mailbox address, if it has one — an opaque,
    /// serialized address `pm-core` supplies and interprets, same
    /// convention as `set_contact_server_addr`.
    pub fn get_own_server_addr(&self) -> Result<Option<Vec<u8>>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT own_server_addr FROM account WHERE id = 0",
                [],
                |row| row.get(0),
            )
            .optional()
            .map(|opt| opt.flatten())
            .map_err(Into::into)
    }

    pub fn set_own_server_addr(&self, addr: Option<&[u8]>) -> Result<()> {
        // The account row might not exist yet if no account pickle has been
        // saved; ensure it does, then set the address (mirrors
        // `set_last_synced_blob_id`).
        self.conn.lock().unwrap().execute(
            "INSERT INTO account (id, pickle, updated_at, own_server_addr)
             VALUES (0, x'', unixepoch(), ?1)
             ON CONFLICT (id) DO UPDATE SET own_server_addr = excluded.own_server_addr",
            params![addr],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const KEY: [u8; 32] = [0x77u8; 32];

    #[test]
    fn contact_roundtrips_and_upsert_is_idempotent_on_identity_key() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let id1 = store
            .upsert_contact(&[1u8; 32], &[2u8; 32], Some("Bob"))
            .unwrap();
        let id2 = store
            .upsert_contact(&[1u8; 32], &[9u8; 32], Some("Bob Renamed"))
            .unwrap();
        assert_eq!(id1, id2, "same identity key must resolve to the same row");
    }

    #[test]
    fn session_pickle_roundtrips() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        assert_eq!(store.load_session_pickle(contact_id).unwrap(), None);

        store
            .save_session_pickle(contact_id, b"pickled-bytes")
            .unwrap();
        assert_eq!(
            store.load_session_pickle(contact_id).unwrap(),
            Some(b"pickled-bytes".to_vec())
        );

        // Saving again replaces rather than erroring or duplicating.
        store
            .save_session_pickle(contact_id, b"new-pickle")
            .unwrap();
        assert_eq!(
            store.load_session_pickle(contact_id).unwrap(),
            Some(b"new-pickle".to_vec())
        );
    }

    #[test]
    fn messages_come_back_in_lamport_order() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();

        for (lamport, text) in [(3u64, "third"), (1, "first"), (2, "second")] {
            store
                .insert_message(
                    contact_id,
                    NewMessage {
                        msg_id: [lamport as u8; 16],
                        direction: Direction::Incoming,
                        lamport,
                        sent_at: 1_754_000_000_000,
                        plaintext: text.as_bytes(),
                        status: None,
                    },
                )
                .unwrap();
        }

        let messages = store.messages_for_contact(contact_id).unwrap();
        let texts: Vec<&str> = messages
            .iter()
            .map(|m| std::str::from_utf8(&m.plaintext).unwrap())
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn data_persists_across_reopening_the_same_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.sqlite");

        let contact_id = {
            let store = Store::open(&path, &KEY).unwrap();
            let contact_id = store
                .upsert_contact(&[1u8; 32], &[2u8; 32], Some("Bob"))
                .unwrap();
            store
                .insert_message(
                    contact_id,
                    NewMessage {
                        msg_id: [9u8; 16],
                        direction: Direction::Outgoing,
                        lamport: 1,
                        sent_at: 1_754_000_000_000,
                        plaintext: b"persisted",
                        status: Some(MessageStatus::Sent),
                    },
                )
                .unwrap();
            contact_id
        }; // store (and its Connection) dropped here

        let reopened = Store::open(&path, &KEY).unwrap();
        let messages = reopened.messages_for_contact(contact_id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].plaintext, b"persisted");
    }

    #[test]
    fn account_pickle_starts_empty_then_roundtrips_and_replaces() {
        let store = Store::open_in_memory(&KEY).unwrap();
        assert_eq!(store.load_account_pickle().unwrap(), None);

        store.save_account_pickle(b"account-v1").unwrap();
        assert_eq!(
            store.load_account_pickle().unwrap(),
            Some(b"account-v1".to_vec())
        );

        store.save_account_pickle(b"account-v2").unwrap();
        assert_eq!(
            store.load_account_pickle().unwrap(),
            Some(b"account-v2".to_vec())
        );
    }

    #[test]
    fn contact_server_addr_starts_unset_then_roundtrips_and_clears() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        assert_eq!(store.get_contact_server_addr(contact_id).unwrap(), None);

        store
            .set_contact_server_addr(contact_id, Some(b"fake-endpoint-addr-bytes"))
            .unwrap();
        assert_eq!(
            store.get_contact_server_addr(contact_id).unwrap(),
            Some(b"fake-endpoint-addr-bytes".to_vec())
        );

        store.set_contact_server_addr(contact_id, None).unwrap();
        assert_eq!(
            store.get_contact_server_addr(contact_id).unwrap(),
            None,
            "clearing back to None must work, not just leave the old value"
        );
    }

    #[test]
    fn pair_secret_and_write_counter_roundtrip() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();

        store
            .set_contact_pair_secret(contact_id, &[5u8; 32])
            .unwrap();
        let contact = store.get_contact(contact_id).unwrap().unwrap();
        assert_eq!(contact.pair_secret, Some([5u8; 32]));
        assert_eq!(contact.next_write_n, 0);

        assert_eq!(store.increment_and_get_next_write_n(contact_id).unwrap(), 0);
        assert_eq!(store.increment_and_get_next_write_n(contact_id).unwrap(), 1);
        assert_eq!(store.increment_and_get_next_write_n(contact_id).unwrap(), 2);
    }

    #[test]
    fn contact_transport_key_starts_unset_then_roundtrips() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        assert_eq!(
            store
                .get_contact(contact_id)
                .unwrap()
                .unwrap()
                .transport_key,
            None
        );

        store
            .set_contact_transport_key(contact_id, &[6u8; 32])
            .unwrap();
        assert_eq!(
            store
                .get_contact(contact_id)
                .unwrap()
                .unwrap()
                .transport_key,
            Some([6u8; 32])
        );
    }

    #[test]
    fn message_status_starts_none_then_roundtrips_and_ignores_unknown_msg_id() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        let msg_id = [3u8; 16];

        store
            .insert_message(
                contact_id,
                NewMessage {
                    msg_id,
                    direction: Direction::Outgoing,
                    lamport: 1,
                    sent_at: 1_754_000_000_000,
                    plaintext: b"hi",
                    status: Some(MessageStatus::Sent),
                },
            )
            .unwrap();
        assert_eq!(
            store.messages_for_contact(contact_id).unwrap()[0].status,
            Some(MessageStatus::Sent)
        );

        store
            .set_message_status(contact_id, &msg_id, MessageStatus::Delivered)
            .unwrap();
        assert_eq!(
            store.messages_for_contact(contact_id).unwrap()[0].status,
            Some(MessageStatus::Delivered)
        );

        // A receipt for a msg_id this contact never actually has is just
        // ignored, not an error — e.g. a stale/duplicate receipt.
        store
            .set_message_status(contact_id, &[99u8; 16], MessageStatus::Delivered)
            .unwrap();
        assert_eq!(store.messages_for_contact(contact_id).unwrap().len(), 1);
    }

    #[test]
    fn set_message_status_by_msg_id_updates_without_knowing_the_contact() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let contact_id = store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        let msg_id = [3u8; 16];
        store
            .insert_message(
                contact_id,
                NewMessage {
                    msg_id,
                    direction: Direction::Outgoing,
                    lamport: 1,
                    sent_at: 1_754_000_000_000,
                    plaintext: b"hi",
                    status: Some(MessageStatus::Sent),
                },
            )
            .unwrap();

        store
            .set_message_status_by_msg_id(&msg_id, MessageStatus::Failed)
            .unwrap();

        assert_eq!(
            store.messages_for_contact(contact_id).unwrap()[0].status,
            Some(MessageStatus::Failed)
        );

        // An unknown msg_id is a harmless no-op, matching set_message_status.
        store
            .set_message_status_by_msg_id(&[99u8; 16], MessageStatus::Failed)
            .unwrap();
    }

    #[test]
    fn list_contacts_returns_everything_in_id_order() {
        let store = Store::open_in_memory(&KEY).unwrap();
        let a = store
            .upsert_contact(&[1u8; 32], &[2u8; 32], Some("A"))
            .unwrap();
        let b = store
            .upsert_contact(&[3u8; 32], &[4u8; 32], Some("B"))
            .unwrap();

        let contacts = store.list_contacts().unwrap();
        assert_eq!(
            contacts.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![a, b]
        );
    }

    #[test]
    fn last_synced_blob_id_defaults_to_zero_and_roundtrips() {
        let store = Store::open_in_memory(&KEY).unwrap();
        assert_eq!(store.get_last_synced_blob_id().unwrap(), 0);

        store.set_last_synced_blob_id(42).unwrap();
        assert_eq!(store.get_last_synced_blob_id().unwrap(), 42);

        // Must not clobber a previously-saved account pickle.
        store.save_account_pickle(b"account-bytes").unwrap();
        store.set_last_synced_blob_id(43).unwrap();
        assert_eq!(
            store.load_account_pickle().unwrap(),
            Some(b"account-bytes".to_vec())
        );
    }

    #[test]
    fn own_server_addr_starts_unset_then_roundtrips_and_clears_without_clobbering_pickle() {
        let store = Store::open_in_memory(&KEY).unwrap();
        assert_eq!(store.get_own_server_addr().unwrap(), None);

        store
            .set_own_server_addr(Some(b"fake-own-endpoint-addr"))
            .unwrap();
        assert_eq!(
            store.get_own_server_addr().unwrap(),
            Some(b"fake-own-endpoint-addr".to_vec())
        );

        // Must not clobber a previously-saved account pickle (same
        // upsert-onto-the-singleton-row pattern as last_synced_blob_id).
        store.save_account_pickle(b"account-bytes").unwrap();
        store.set_own_server_addr(Some(b"new-addr")).unwrap();
        assert_eq!(
            store.load_account_pickle().unwrap(),
            Some(b"account-bytes".to_vec())
        );

        store.set_own_server_addr(None).unwrap();
        assert_eq!(
            store.get_own_server_addr().unwrap(),
            None,
            "clearing back to None must work, not just leave the old value"
        );
    }

    #[test]
    fn opening_with_the_wrong_key_fails_to_read_existing_data() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.sqlite");

        {
            let store = Store::open(&path, &KEY).unwrap();
            store.upsert_contact(&[1u8; 32], &[2u8; 32], None).unwrap();
        }

        // SQLCipher accepts any key at open() (it doesn't know it's wrong
        // yet), so the failure surfaces on first real read/write against the
        // now-undecryptable pages.
        let wrong_key = [0x99u8; 32];
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex::encode(wrong_key)))
            .unwrap();
        let result: rusqlite::Result<i64> =
            conn.query_row("SELECT count(*) FROM contacts", [], |row| row.get(0));
        assert!(
            result.is_err(),
            "reading with the wrong key must fail, not silently return garbage or nothing"
        );
    }
}
