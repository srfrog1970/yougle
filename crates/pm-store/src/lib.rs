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

use rusqlite::{params, Connection, OptionalExtension};

use error::Result;
pub use error::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub struct NewMessage<'a> {
    pub msg_id: [u8; 16],
    pub direction: Direction,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub msg_id: [u8; 16],
    pub direction: Direction,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: Vec<u8>,
}

/// A single user's encrypted local database: contacts, session state, and
/// message history.
pub struct Store {
    conn: Connection,
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
        Ok(Self { conn })
    }

    /// Inserts a contact, or updates its keys/name if the identity key is
    /// already known. Returns the contact's local row id.
    pub fn upsert_contact(
        &self,
        identity_key: &[u8; 32],
        curve25519_key: &[u8; 32],
        display_name: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
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
        self.conn
            .query_row(
                "SELECT id FROM contacts WHERE identity_key = ?1",
                [identity_key.as_slice()],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    /// Saves (or replaces) the pickled Olm session state for a contact.
    pub fn save_session_pickle(&self, contact_id: i64, pickle: &[u8]) -> Result<()> {
        self.conn.execute(
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
            .query_row(
                "SELECT pickle FROM sessions WHERE contact_id = ?1",
                [contact_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn insert_message(&self, contact_id: i64, msg: NewMessage) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (contact_id, msg_id, direction, lamport, sent_at, plaintext, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())",
            params![
                contact_id,
                msg.msg_id.as_slice(),
                msg.direction.as_str(),
                msg.lamport as i64,
                msg.sent_at as i64,
                msg.plaintext,
            ],
        )?;
        Ok(())
    }

    /// All messages with a contact, oldest first by Lamport order.
    pub fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT msg_id, direction, lamport, sent_at, plaintext
             FROM messages WHERE contact_id = ?1 ORDER BY lamport ASC",
        )?;
        let rows = stmt.query_map([contact_id], |row| {
            let msg_id: Vec<u8> = row.get(0)?;
            let direction: String = row.get(1)?;
            let lamport: i64 = row.get(2)?;
            let sent_at: i64 = row.get(3)?;
            let plaintext: Vec<u8> = row.get(4)?;
            Ok((msg_id, direction, lamport, sent_at, plaintext))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (msg_id, direction, lamport, sent_at, plaintext) = row?;
            let msg_id: [u8; 16] = msg_id
                .try_into()
                .map_err(|v: Vec<u8>| StoreError::InvalidMsgIdLength(v.len()))?;
            out.push(StoredMessage {
                msg_id,
                direction: Direction::from_str(&direction),
                lamport: lamport as u64,
                sent_at: sent_at as u64,
                plaintext,
            });
        }
        Ok(out)
    }

    /// Advances this device's Lamport clock for a local event (e.g., sending
    /// a message) and returns the new value.
    pub fn tick_lamport(&self) -> Result<u64> {
        lamport::tick(&self.conn)
    }

    /// Merges an observed remote Lamport value (e.g., from a received
    /// message) into this device's clock and returns the new local value.
    pub fn observe_lamport(&self, remote: u64) -> Result<u64> {
        lamport::observe(&self.conn, remote)
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
