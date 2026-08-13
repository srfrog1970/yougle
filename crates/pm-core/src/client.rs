//! The client API: ties `pm-crypto` (identity, sessions), `pm-store`
//! (persistence), `pm-transport` (network), and `pm-proto` (wire framing)
//! into one coherent surface.
//!
//! **Scope note (M3):** only the Server-mailbox delivery path is wired up
//! here. Direct Local (phone-to-phone) delivery already works at the
//! `pm-transport`/`pm-crypto` level (proven in the M1/M2 tests) but isn't
//! integrated into `Client` yet — that needs `Client` to also run its own
//! accept loop while foregrounded, which is deferred to M4/M5 when the app
//! actually needs both modes. `send`/`sync` here require a contact/self to
//! have a known Server address.

use std::path::Path;

use rand::RngCore;
use sha2::{Digest, Sha256};

use iroh::EndpointAddr;
use pm_crypto::{Identity, MyAccount, MySession, Seed};
use pm_proto::{Envelope, NodeRequest, NodeResponse};
use pm_store::{ContactRecord, Direction, NewMessage, Store};
use pm_transport::NodeClient;

use crate::backup::{self, BackupBundle};
use crate::error::{CoreError, Result};

/// How many upcoming write-auth slots to pre-register at once. A real
/// deployment would replenish this in the background as it runs low;
/// fixed and small here since M3's job is proving the mechanism works, not
/// tuning it (see `docs/PRD.md`'s "Retry semantics will be a setting" for
/// the analogous, still-open question on the send-retry side).
const SLOT_REPLENISH_BATCH: u64 = 8;

pub struct Client {
    identity: Identity,
    store: Store,
    account: MyAccount,
    node_client: NodeClient,
    server_addr: Option<EndpointAddr>,
}

impl Client {
    /// Opens (creating if needed) a client backed by the store at
    /// `store_path`, for the identity derived from `seed`. `server_addr` is
    /// this device's own Server mailbox, if it has one — re-supplied by the
    /// caller each session, since there's no DHT lookup in this build (see
    /// `docs/PRD.md`'s Open Items).
    pub async fn open(
        seed: &Seed,
        store_path: &Path,
        server_addr: Option<EndpointAddr>,
    ) -> Result<Self> {
        let identity = Identity::derive(seed);
        let store = Store::open(store_path, &identity.mailbox_key)?;

        let account = match store.load_account_pickle()? {
            Some(bytes) => MyAccount::from_pickle(&bytes)?,
            None => {
                let fresh = MyAccount::new();
                store.save_account_pickle(&fresh.pickle())?;
                fresh
            }
        };

        let node_client = NodeClient::new().await?;

        Ok(Self {
            identity,
            store,
            account,
            node_client,
            server_addr,
        })
    }

    pub fn identity_key(&self) -> [u8; 32] {
        self.identity.signing_key.verifying_key().to_bytes()
    }

    pub fn curve25519_key(&self) -> [u8; 32] {
        self.account.curve25519_key().to_bytes()
    }

    /// Generates and returns one of this client's vodozemac one-time keys,
    /// ready to hand to a pairing partner (stand-in for what a real QR
    /// payload would carry — pairing/QR itself isn't built yet).
    pub fn generate_one_time_key(&mut self) -> Result<[u8; 32]> {
        let keys = self.account.generate_one_time_keys(1);
        self.store.save_account_pickle(&self.account.pickle())?;
        Ok(*keys
            .values()
            .next()
            .expect("just generated exactly one key")
            .as_bytes())
    }

    /// Records a new contact and establishes this client's outbound
    /// session to them immediately, using one of their published one-time
    /// keys — stand-in for mutual QR pairing, per `pm-crypto`/`pm-proto`'s
    /// M1/M2 tests. `pair_secret` stands in for the real per-pair secret a
    /// mutual QR scan would derive (`ARCHIT_1.MD` §4.2); if this client has
    /// its own Server, a batch of upcoming write-auth slots for this
    /// contact is pre-registered there too.
    pub async fn add_contact(
        &mut self,
        their_identity_key: [u8; 32],
        their_curve25519_key: [u8; 32],
        their_one_time_key: [u8; 32],
        display_name: Option<&str>,
        their_server_addr: Option<EndpointAddr>,
        pair_secret: [u8; 32],
    ) -> Result<i64> {
        let contact_id =
            self.store
                .upsert_contact(&their_identity_key, &their_curve25519_key, display_name)?;
        self.store
            .set_contact_pair_secret(contact_id, &pair_secret)?;

        if let Some(addr) = &their_server_addr {
            let bytes =
                bincode::serialize(addr).map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
            self.store
                .set_contact_server_addr(contact_id, Some(&bytes))?;
        }

        // The outbound session is established lazily, on this device's
        // first actual send to them (see `send`) — not here. Olm has
        // exactly one session per pair; eagerly creating an outbound
        // session on both sides at pairing time would create two
        // independent, mismatched sessions instead of one shared one.
        self.store
            .set_contact_pending_otk(contact_id, Some(&their_one_time_key))?;

        if let Some(my_addr) = self.server_addr.clone() {
            for n in 0..SLOT_REPLENISH_BATCH {
                let auth = derive_auth(&pair_secret, n)?;
                let slot_hash: [u8; 32] = Sha256::digest(auth).into();
                self.register_slot(my_addr.clone(), slot_hash).await?;
            }
        }

        Ok(contact_id)
    }

    async fn register_slot(&self, node_addr: EndpointAddr, slot_hash: [u8; 32]) -> Result<()> {
        let response = self
            .node_client
            .call(
                node_addr,
                &NodeRequest::RegisterSlot {
                    mailbox_key: self.identity.mailbox_key,
                    slot_hash,
                },
            )
            .await?;
        expect_ok(response)
    }

    /// Encrypts and sends a message to a contact, via that contact's Server
    /// mailbox (see the module-level scope note re: Local delivery).
    pub async fn send(&mut self, contact_id: i64, plaintext: &[u8]) -> Result<()> {
        let contact = self.get_contact(contact_id)?;
        let server_addr_bytes = contact.server_addr.ok_or(CoreError::NoServerForContact)?;
        let their_server_addr: EndpointAddr = bincode::deserialize(&server_addr_bytes)
            .map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
        let pair_secret = contact.pair_secret.ok_or(CoreError::NoServerForContact)?;

        let mut session = match self.store.load_session_pickle(contact_id)? {
            Some(pickle) => MySession::from_pickle(&pickle)?,
            None => {
                // First send to this contact: lazily establish the
                // outbound session now, consuming the one-time key they
                // gave us at pairing time.
                let their_otk = contact.pending_otk.ok_or(CoreError::NoServerForContact)?;
                let their_curve =
                    vodozemac::Curve25519PublicKey::from_bytes(contact.curve25519_key);
                let their_otk = vodozemac::Curve25519PublicKey::from_bytes(their_otk);
                let session = self
                    .account
                    .create_outbound_session(their_curve, their_otk)?;
                self.store.set_contact_pending_otk(contact_id, None)?;
                session
            }
        };

        let (olm_type, ciphertext) = session.encrypt(plaintext)?;

        let n = self.store.increment_and_get_next_write_n(contact_id)?;
        let auth = derive_auth(&pair_secret, n)?;

        let mut msg_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut msg_id);
        let sent_at = now_millis();
        let lamport = self.store.tick_lamport()?;

        let envelope = Envelope::new(olm_type, ciphertext, n, lamport, sent_at, msg_id);
        let envelope_bytes = envelope.to_padded_bytes()?;

        let response = self
            .node_client
            .call(
                their_server_addr,
                &NodeRequest::Write {
                    auth,
                    blob: envelope_bytes,
                },
            )
            .await?;
        expect_ok(response)?;

        self.store
            .save_session_pickle(contact_id, &session.pickle())?;
        self.store.insert_message(
            contact_id,
            NewMessage {
                msg_id,
                direction: Direction::Outgoing,
                lamport,
                sent_at,
                plaintext,
            },
        )?;

        Ok(())
    }

    /// Fetches new messages from this client's own Server, attributes each
    /// to a known contact by trying to decrypt against each one in turn
    /// (safe: a wrong attempt just fails, per Olm's authenticated
    /// encryption — no separate sender-identity field is needed on the
    /// wire), persists them, and acks. Returns how many new messages were
    /// processed.
    pub async fn sync(&mut self) -> Result<usize> {
        let my_addr = self.server_addr.clone().ok_or(CoreError::NoOwnServer)?;
        let response = self
            .node_client
            .call(
                my_addr,
                &NodeRequest::Fetch {
                    mailbox_key: self.identity.mailbox_key,
                },
            )
            .await?;
        let NodeResponse::Blobs(blobs) = response else {
            return Err(unexpected_response(response));
        };

        let watermark = self.store.get_last_synced_blob_id()?;
        let mut new_blobs: Vec<_> = blobs.into_iter().filter(|b| b.id > watermark).collect();
        new_blobs.sort_by_key(|b| b.id);

        let mut processed = 0;
        let mut max_id_seen = watermark;
        let mut acked_ids = Vec::new();

        for blob in new_blobs {
            max_id_seen = max_id_seen.max(blob.id);
            let Ok(envelope) = Envelope::from_padded_bytes(&blob.blob) else {
                continue; // not one of ours / malformed — skip, don't fail the whole sync
            };
            if let Some(contact_id) = self.attribute_and_store(&envelope)? {
                let _ = contact_id;
                processed += 1;
                acked_ids.push(blob.id);
            }
        }

        if !acked_ids.is_empty() {
            let response = self
                .node_client
                .call(
                    self.server_addr.clone().unwrap(),
                    &NodeRequest::Ack {
                        mailbox_key: self.identity.mailbox_key,
                        ids: acked_ids,
                    },
                )
                .await?;
            expect_ok(response)?;
        }
        self.store.set_last_synced_blob_id(max_id_seen)?;

        Ok(processed)
    }

    /// Tries each known contact's session (or, if none is established yet,
    /// tries establishing an inbound one from this being their first
    /// message) until one successfully decrypts the envelope.
    fn attribute_and_store(&mut self, envelope: &Envelope) -> Result<Option<i64>> {
        let contacts = self.store.list_contacts()?;
        for contact in contacts {
            if let Some(pickle) = self.store.load_session_pickle(contact.id)? {
                let mut session = MySession::from_pickle(&pickle)?;
                if let Ok(plaintext) = session.decrypt(envelope.olm_type, &envelope.ciphertext) {
                    self.store
                        .save_session_pickle(contact.id, &session.pickle())?;
                    self.persist_incoming(contact.id, envelope, &plaintext)?;
                    return Ok(Some(contact.id));
                }
            } else {
                let their_curve =
                    vodozemac::Curve25519PublicKey::from_bytes(contact.curve25519_key);
                if let Ok((session, plaintext)) = self.account.accept_incoming(
                    their_curve,
                    envelope.olm_type,
                    &envelope.ciphertext,
                ) {
                    self.store.save_account_pickle(&self.account.pickle())?;
                    self.store
                        .save_session_pickle(contact.id, &session.pickle())?;
                    self.persist_incoming(contact.id, envelope, &plaintext)?;
                    return Ok(Some(contact.id));
                }
            }
        }
        Ok(None)
    }

    fn persist_incoming(
        &self,
        contact_id: i64,
        envelope: &Envelope,
        plaintext: &[u8],
    ) -> Result<()> {
        let lamport = self.store.observe_lamport(envelope.lamport)?;
        self.store.insert_message(
            contact_id,
            NewMessage {
                msg_id: envelope.msg_id,
                direction: Direction::Incoming,
                lamport,
                sent_at: envelope.sent_at,
                plaintext,
            },
        )?;
        Ok(())
    }

    pub fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<pm_store::StoredMessage>> {
        Ok(self.store.messages_for_contact(contact_id)?)
    }

    fn get_contact(&self, contact_id: i64) -> Result<ContactRecord> {
        self.store
            .get_contact(contact_id)?
            .ok_or(CoreError::UnknownContact(contact_id))
    }

    /// Assembles, encrypts under this identity's backup key, and pushes the
    /// backup bundle (contacts, session state, own account) to this
    /// client's own Server. Requires `server_addr` to have been configured.
    pub async fn push_backup(&self) -> Result<()> {
        let my_addr = self.server_addr.clone().ok_or(CoreError::NoOwnServer)?;
        let bundle = backup::assemble(&self.store)?;
        let plaintext = backup::serialize(&bundle);
        let ciphertext = pm_crypto::encrypt_backup(&self.identity.backup_key, &plaintext);

        let response = self
            .node_client
            .call(
                my_addr,
                &NodeRequest::PutBackup {
                    mailbox_key: self.identity.mailbox_key,
                    blob: ciphertext,
                },
            )
            .await?;
        expect_ok(response)
    }

    /// Restores identity (always possible from the seed alone) plus
    /// contacts and message-adjacent session state (only possible here by
    /// fetching a previously pushed backup from `server_addr`, which the
    /// caller must already know — see the module-level scope note on why
    /// this isn't a DHT lookup). Opens a fresh store at `store_path`, which
    /// must not already exist.
    pub async fn restore(
        seed: &Seed,
        store_path: &Path,
        server_addr: EndpointAddr,
    ) -> Result<Self> {
        let identity = Identity::derive(seed);
        let node_client = NodeClient::new().await?;

        let response = node_client
            .call(
                server_addr.clone(),
                &NodeRequest::GetBackup {
                    mailbox_key: identity.mailbox_key,
                },
            )
            .await?;
        let NodeResponse::Backup(maybe_blob) = response else {
            return Err(unexpected_response(response));
        };
        let ciphertext = maybe_blob.ok_or(CoreError::NoBackupFound)?;
        let plaintext = pm_crypto::decrypt_backup(&identity.backup_key, &ciphertext)?;
        let bundle: BackupBundle = backup::deserialize(&plaintext)?;

        let store = Store::open(store_path, &identity.mailbox_key)?;
        backup::restore_into(&store, &bundle)?;

        let account = MyAccount::from_pickle(&bundle.account_pickle)?;

        Ok(Self {
            identity,
            store,
            account,
            node_client,
            server_addr: Some(server_addr),
        })
    }
}

fn derive_auth(pair_secret: &[u8; 32], n: u64) -> Result<[u8; 32]> {
    let bytes = pm_proto::derive(pair_secret, "auth", n, 32)?;
    Ok(bytes
        .try_into()
        .expect("derive(.., 32) always returns 32 bytes"))
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_millis() as u64
}

fn expect_ok(response: NodeResponse) -> Result<()> {
    match response {
        NodeResponse::Ok => Ok(()),
        other => Err(unexpected_response(other)),
    }
}

fn unexpected_response(response: NodeResponse) -> CoreError {
    match response {
        NodeResponse::Error(e) => CoreError::NodeError(e),
        other => CoreError::NodeError(format!("unexpected response: {other:?}")),
    }
}
