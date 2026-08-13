//! Assembles/restores the backup bundle contents (what `pm_crypto::backup`
//! encrypts/decrypts as opaque bytes). Covers contacts, per-contact session
//! and pairing state, and this device's own vodozemac account — everything
//! needed to resume conversations on a new device, per `docs/PRD.md`'s
//! "Recovery" capability.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupContact {
    pub identity_key: [u8; 32],
    pub curve25519_key: [u8; 32],
    pub display_name: Option<String>,
    pub server_addr: Option<Vec<u8>>,
    pub pair_secret: Option<[u8; 32]>,
    pub next_write_n: u64,
    pub session_pickle: Option<Vec<u8>>,
    pub pending_otk: Option<[u8; 32]>,
    pub messages: Vec<pm_store::StoredMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupBundle {
    pub account_pickle: Vec<u8>,
    pub contacts: Vec<BackupContact>,
    /// This device's own Server mailbox address at backup time, if it had
    /// one — so restoring on a new device remembers it too, rather than
    /// requiring it to be re-entered (see `pm-core`'s M5 `Client` changes).
    pub own_server_addr: Option<Vec<u8>>,
}

pub fn assemble(store: &pm_store::Store) -> Result<BackupBundle> {
    let account_pickle = store
        .load_account_pickle()?
        .ok_or_else(|| CoreError::CorruptBackup("no account pickle to back up".to_string()))?;
    let own_server_addr = store.get_own_server_addr()?;

    let mut contacts = Vec::new();
    for c in store.list_contacts()? {
        let session_pickle = store.load_session_pickle(c.id)?;
        let messages = store.messages_for_contact(c.id)?;
        contacts.push(BackupContact {
            identity_key: c.identity_key,
            curve25519_key: c.curve25519_key,
            display_name: c.display_name,
            server_addr: c.server_addr,
            pair_secret: c.pair_secret,
            next_write_n: c.next_write_n,
            session_pickle,
            pending_otk: c.pending_otk,
            messages,
        });
    }

    Ok(BackupBundle {
        account_pickle,
        contacts,
        own_server_addr,
    })
}

/// Restores a bundle into a fresh (empty) store.
pub fn restore_into(store: &pm_store::Store, bundle: &BackupBundle) -> Result<()> {
    store.save_account_pickle(&bundle.account_pickle)?;
    store.set_own_server_addr(bundle.own_server_addr.as_deref())?;

    for c in &bundle.contacts {
        let contact_id = store.upsert_contact(
            &c.identity_key,
            &c.curve25519_key,
            c.display_name.as_deref(),
        )?;
        if let Some(addr) = &c.server_addr {
            store.set_contact_server_addr(contact_id, Some(addr))?;
        }
        if let Some(secret) = &c.pair_secret {
            store.set_contact_pair_secret(contact_id, secret)?;
        }
        for _ in 0..c.next_write_n {
            store.increment_and_get_next_write_n(contact_id)?;
        }
        if let Some(pickle) = &c.session_pickle {
            store.save_session_pickle(contact_id, pickle)?;
        }
        if let Some(otk) = &c.pending_otk {
            store.set_contact_pending_otk(contact_id, Some(otk))?;
        }
        for m in &c.messages {
            store.insert_message(
                contact_id,
                pm_store::NewMessage {
                    msg_id: m.msg_id,
                    direction: m.direction,
                    lamport: m.lamport,
                    sent_at: m.sent_at,
                    plaintext: &m.plaintext,
                    status: m.status,
                },
            )?;
        }
    }

    Ok(())
}

pub fn serialize(bundle: &BackupBundle) -> Vec<u8> {
    bincode::serialize(bundle).expect("BackupBundle serialization cannot fail")
}

pub fn deserialize(bytes: &[u8]) -> Result<BackupBundle> {
    bincode::deserialize(bytes).map_err(|e| CoreError::CorruptBackup(e.to_string()))
}
