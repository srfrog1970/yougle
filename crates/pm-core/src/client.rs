//! The client API: ties `pm-crypto` (identity, sessions), `pm-store`
//! (persistence), `pm-transport` (network), and `pm-proto` (wire framing)
//! into one coherent surface.
//!
//! `Client` is a cheap-to-`Clone` handle around `Arc<ClientShared>` (plus
//! the `Router` running this device's own Local-delivery accept loop —
//! see M6). This isn't just an API nicety: the accept loop's handler
//! (`DirectHandler`) needs to call back into exactly the same account/
//! session state the foreground API mutates (`account.accept_incoming`,
//! one-time-key consumption), from a background task running concurrently
//! with whatever the foreground caller is doing. `pm_store::Store` was
//! already made `Sync` for this reason back in M4 (uniffi's async
//! bridging needed it); this carries that same fix one layer deeper, since
//! now there's a genuine background task, not just cross-thread FFI
//! access — every method moved from `&mut self` to `&self` accordingly.
//!
//! `DirectHandler` holds only a `Weak<ClientShared>`, not a full `Client`
//! (which would also hold a strong reference to the very `Router` that
//! owns the handler — a reference cycle, since `Router`'s background task
//! stays alive via an `AbortOnDropHandle` reachable only through a live
//! `Client`). `Weak` breaks that cycle: once every `Client` handle is
//! dropped, `Router` (and its `AbortOnDropHandle`) drops too, which stops
//! the accept loop — matching `docs/PRD.md`'s "opening the app only makes
//! the user reachable going forward" (nothing keeps this device reachable
//! once nothing holds a `Client` anymore).

use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{EndpointAddr, EndpointId};
use rand::RngCore;
use sha2::{Digest, Sha256};

use pm_crypto::{Identity, MyAccount, MySession, Seed};
use pm_proto::{DirectAck, Envelope, NodeRequest, NodeResponse, DIRECT_ALPN};
use pm_store::{ContactRecord, Direction, MessageStatus, NewMessage, Store};
use pm_transport::NodeClient;

use crate::backup::{self, BackupBundle};
use crate::error::{CoreError, Result};
use crate::message::PlaintextPayload;
use crate::pairing::{compute_pair_secret, PairingPayload};

/// How many upcoming write-auth slots to pre-register at once. A real
/// deployment would replenish this in the background as it runs low;
/// fixed and small here since M3's job is proving the mechanism works, not
/// tuning it (see `docs/PRD.md`'s "Retry semantics will be a setting" for
/// the analogous, still-open question on the send-retry side). This
/// budget is shared between real chat messages *and* the delivery
/// receipts M6 introduces (both are just writes against this same
/// registered sequence) — an existing, documented limitation this doesn't
/// newly introduce, just reaches sooner.
const SLOT_REPLENISH_BATCH: u64 = 8;

/// How long a direct P2P delivery attempt waits before giving up — per
/// `docs/PRD.md` Flow 3's "~15-20 seconds".
const DIRECT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(20);

struct ClientShared {
    identity: Identity,
    store: Store,
    account: Mutex<MyAccount>,
    node_client: NodeClient,
    server_addr: Mutex<Option<EndpointAddr>>,
    /// Serializes `deliver()` calls. M6 introduced a second caller of
    /// `deliver()` beyond the foreground `send()` — the best-effort receipt
    /// spawned by `handle_decrypted` — so two deliveries (e.g. a reply and
    /// a receipt-for-the-message-that-triggered-it) can now genuinely race
    /// each other from separate tokio tasks. Without this, both can load
    /// the same contact's Olm session pickle before either saves it back,
    /// silently corrupting that session's ratchet state (each encrypt
    /// mutates it in memory) — no error at send time, just an
    /// unattributable envelope on the receiving end later. Coarse (one
    /// lock per `Client`, not per contact) on purpose: correctness over
    /// throughput for now, matching this build's existing "prove the
    /// mechanism, don't tune it" stance elsewhere (see
    /// `SLOT_REPLENISH_BATCH`).
    send_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct Client {
    shared: Arc<ClientShared>,
    /// Keeps this device's Local-delivery accept loop running for as long
    /// as any `Client` handle exists — see the module doc comment.
    _direct_router: Router,
}

impl Client {
    /// Opens (creating if needed) a client backed by the store at
    /// `store_path`, for the identity derived from `seed`. This device's own
    /// Server mailbox address, if any, is loaded from the store itself
    /// (`None` on a fresh store) — see `own_server_addr`/`set_own_server_addr`
    /// to configure it; there's no DHT lookup in this build (see
    /// `docs/PRD.md`'s Open Items), so it's this device's own responsibility
    /// to remember, not looked up externally.
    pub async fn open(seed: &Seed, store_path: &Path) -> Result<Self> {
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

        let server_addr = load_own_server_addr(&store)?;
        let node_client = NodeClient::new(identity.transport_key).await?;

        Self::from_parts(identity, store, account, node_client, server_addr).await
    }

    /// Shared by every constructor: wraps the assembled pieces in the
    /// `Arc`, spawns the Local-delivery accept loop around a clone of
    /// `node_client`'s own endpoint (see `NodeClient::endpoint`'s docs for
    /// why that's safe to share), and returns the resulting handle.
    async fn from_parts(
        identity: Identity,
        store: Store,
        account: MyAccount,
        node_client: NodeClient,
        server_addr: Option<EndpointAddr>,
    ) -> Result<Self> {
        let shared = Arc::new(ClientShared {
            identity,
            store,
            account: Mutex::new(account),
            node_client,
            server_addr: Mutex::new(server_addr),
            send_lock: tokio::sync::Mutex::new(()),
        });

        let handler = DirectHandler {
            shared: Arc::downgrade(&shared),
        };
        let endpoint = shared.node_client.endpoint();
        let direct_router = Router::builder(endpoint)
            .accept(DIRECT_ALPN, handler)
            .spawn();

        Ok(Self {
            shared,
            _direct_router: direct_router,
        })
    }

    /// This device's own Server mailbox address, if it has configured one.
    pub fn own_server_addr(&self) -> Option<EndpointAddr> {
        self.shared.server_addr.lock().unwrap().clone()
    }

    /// Configures this device's own Server mailbox. Persisted immediately,
    /// so it's remembered across app restarts without needing to be
    /// re-entered (see `open`).
    pub fn set_own_server_addr(&self, addr: EndpointAddr) -> Result<()> {
        let bytes =
            bincode::serialize(&addr).map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
        self.shared.store.set_own_server_addr(Some(&bytes))?;
        *self.shared.server_addr.lock().unwrap() = Some(addr);
        Ok(())
    }

    /// Removes this device's own Server mailbox configuration, reverting to
    /// Local-only.
    pub fn clear_own_server_addr(&self) -> Result<()> {
        self.shared.store.set_own_server_addr(None)?;
        *self.shared.server_addr.lock().unwrap() = None;
        Ok(())
    }

    pub fn list_contacts(&self) -> Result<Vec<ContactRecord>> {
        Ok(self.shared.store.list_contacts()?)
    }

    pub fn identity_key(&self) -> [u8; 32] {
        self.shared.identity.signing_key.verifying_key().to_bytes()
    }

    pub fn curve25519_key(&self) -> [u8; 32] {
        self.shared
            .account
            .lock()
            .unwrap()
            .curve25519_key()
            .to_bytes()
    }

    /// Generates and returns one of this client's vodozemac one-time keys,
    /// ready to hand to a pairing partner (see `pairing_payload`).
    pub fn generate_one_time_key(&self) -> Result<[u8; 32]> {
        self.shared.generate_one_time_key()
    }

    /// Records a new contact and stores everything needed to reach and
    /// message them — their signing/session keys, their iroh transport key
    /// (for Local delivery — see M6), and, if they have one, their Server
    /// mailbox address. `pair_secret` stands in for the real per-pair
    /// secret a mutual QR scan derives (`compute_pair_secret`); if this
    /// client has its own Server, a batch of upcoming write-auth slots for
    /// this contact is pre-registered there too.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_contact(
        &self,
        their_identity_key: [u8; 32],
        their_curve25519_key: [u8; 32],
        their_one_time_key: [u8; 32],
        their_transport_key: [u8; 32],
        display_name: Option<&str>,
        their_server_addr: Option<EndpointAddr>,
        pair_secret: [u8; 32],
    ) -> Result<i64> {
        self.shared
            .add_contact(
                their_identity_key,
                their_curve25519_key,
                their_one_time_key,
                their_transport_key,
                display_name,
                their_server_addr,
                pair_secret,
            )
            .await
    }

    /// Produces this device's shareable pairing data — public keys, a fresh
    /// one-time key, and a fresh random nonce — for a partner to scan/paste
    /// (see `PairingPayload`). Hold onto the returned nonce: it's needed
    /// again, unchanged, when this same pairing attempt is completed via
    /// `add_contact_from_payload`.
    pub fn pairing_payload(&self) -> Result<PairingPayload> {
        let one_time_key = self.generate_one_time_key()?;
        let mut nonce = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let server_addr = self
            .own_server_addr()
            .as_ref()
            .map(bincode::serialize)
            .transpose()
            .map_err(|e| CoreError::CorruptBackup(e.to_string()))?;

        Ok(PairingPayload {
            identity_key: self.identity_key(),
            curve25519_key: self.curve25519_key(),
            // The *public* iroh id, not `identity.transport_key` itself —
            // that's this device's private endpoint secret (what seeds
            // `NodeClient::new`), and must never leave the device. A
            // contact needs the public id to dial in via
            // `EndpointId::from_bytes`/`EndpointAddr::from` (see `deliver`'s
            // direct-delivery branch).
            transport_key: *self.shared.node_client.endpoint().id().as_bytes(),
            one_time_key,
            nonce,
            server_addr,
        })
    }

    /// Completes a pairing exchange: `their` is the partner's
    /// `pairing_payload()`, and `my_nonce` is the nonce from *this*
    /// device's own `pairing_payload()` call for the same attempt (not
    /// re-generated here) — combining the two nonces via
    /// `compute_pair_secret` is what lets both sides land on the identical
    /// `pair_secret` regardless of who calls this first.
    pub async fn add_contact_from_payload(
        &self,
        their: PairingPayload,
        my_nonce: [u8; 32],
        display_name: Option<&str>,
    ) -> Result<i64> {
        let pair_secret = compute_pair_secret(my_nonce, their.nonce)?;
        let their_server_addr = their
            .server_addr
            .as_deref()
            .map(bincode::deserialize)
            .transpose()
            .map_err(|e: bincode::Error| CoreError::CorruptBackup(e.to_string()))?;

        self.add_contact(
            their.identity_key,
            their.curve25519_key,
            their.one_time_key,
            their.transport_key,
            display_name,
            their_server_addr,
            pair_secret,
        )
        .await
    }

    /// Encrypts and delivers a message to a contact — via their Server
    /// mailbox if they have one on file, otherwise a direct Local P2P
    /// attempt, falling back (M7) to this device's own Server retrying on
    /// a schedule if that attempt fails and one is configured
    /// (`docs/PRD.md` Flow 3's all three cases). Persists the message with
    /// whatever status delivery actually achieved — `Sent` (written to
    /// their Server, or queued on this device's own Server for retry;
    /// `Delivered` follows later via a receipt, or `Failed` if the retry
    /// schedule exhausts) or `Delivered` directly (a successful direct
    /// attempt is already synchronous confirmation). A failed attempt with
    /// no Server to fall back to returns an error and persists nothing,
    /// leaving the caller free to retry — same "revert to compose box"
    /// behavior the app's UI already implements for any `send` error.
    pub async fn send(&self, contact_id: i64, plaintext: &[u8]) -> Result<()> {
        self.shared.send(contact_id, plaintext).await
    }

    /// Fetches new messages from this client's own Server, attributes each
    /// to a known contact, persists real chat messages, applies delivery
    /// receipts to the messages they're for, and acks. Returns how many
    /// new *chat* messages were processed (receipts aren't counted — they
    /// update existing messages, not add new ones).
    pub async fn sync(&self) -> Result<usize> {
        ClientShared::sync(&self.shared).await
    }

    pub fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<pm_store::StoredMessage>> {
        Ok(self.shared.store.messages_for_contact(contact_id)?)
    }

    /// Assembles, encrypts under this identity's backup key, and pushes the
    /// backup bundle (contacts, session state, own account) to this
    /// client's own Server. Requires `server_addr` to have been configured.
    pub async fn push_backup(&self) -> Result<()> {
        self.shared.push_backup().await
    }

    /// Assembles and encrypts the same backup bundle as `push_backup`, but
    /// returns the ciphertext directly instead of pushing it to a Server —
    /// works with no Server mailbox configured at all, for the manual
    /// export/import path (`docs/PRD.md` §7's "Backup export screen").
    pub fn export_backup(&self) -> Result<Vec<u8>> {
        self.shared.export_backup()
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
        let node_client = NodeClient::new(identity.transport_key).await?;

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
        // The address just used to fetch this backup is, by construction,
        // this device's own Server — persist it so a later plain `open()`
        // (not `restore()`) remembers it too, overriding whatever the
        // bundle itself happened to record (should coincide, but this is
        // the address that's actually known-reachable right now).
        let addr_bytes = bincode::serialize(&server_addr)
            .map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
        store.set_own_server_addr(Some(&addr_bytes))?;

        let account = MyAccount::from_pickle(&bundle.account_pickle)?;

        Self::from_parts(identity, store, account, node_client, Some(server_addr)).await
    }

    /// Restores identity plus contacts/message history from a manually
    /// supplied encrypted backup (as produced by `export_backup`), rather
    /// than fetching one from a Server — works with no Server mailbox at
    /// all. Opens a fresh store at `store_path`, which must not already
    /// exist.
    pub async fn import_backup(
        seed: &Seed,
        store_path: &Path,
        backup_bytes: &[u8],
    ) -> Result<Self> {
        let identity = Identity::derive(seed);
        let plaintext = pm_crypto::decrypt_backup(&identity.backup_key, backup_bytes)?;
        let bundle: BackupBundle = backup::deserialize(&plaintext)?;

        let store = Store::open(store_path, &identity.mailbox_key)?;
        backup::restore_into(&store, &bundle)?;

        let account = MyAccount::from_pickle(&bundle.account_pickle)?;
        let server_addr = load_own_server_addr(&store)?;
        let node_client = NodeClient::new(identity.transport_key).await?;

        Self::from_parts(identity, store, account, node_client, server_addr).await
    }
}

impl ClientShared {
    fn generate_one_time_key(&self) -> Result<[u8; 32]> {
        let mut account = self.account.lock().unwrap();
        let keys = account.generate_one_time_keys(1);
        self.store.save_account_pickle(&account.pickle())?;
        Ok(*keys
            .values()
            .next()
            .expect("just generated exactly one key")
            .as_bytes())
    }

    #[allow(clippy::too_many_arguments)]
    async fn add_contact(
        &self,
        their_identity_key: [u8; 32],
        their_curve25519_key: [u8; 32],
        their_one_time_key: [u8; 32],
        their_transport_key: [u8; 32],
        display_name: Option<&str>,
        their_server_addr: Option<EndpointAddr>,
        pair_secret: [u8; 32],
    ) -> Result<i64> {
        let contact_id =
            self.store
                .upsert_contact(&their_identity_key, &their_curve25519_key, display_name)?;
        self.store
            .set_contact_pair_secret(contact_id, &pair_secret)?;
        self.store
            .set_contact_transport_key(contact_id, &their_transport_key)?;

        if let Some(addr) = &their_server_addr {
            let bytes =
                bincode::serialize(addr).map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
            self.store
                .set_contact_server_addr(contact_id, Some(&bytes))?;
        }

        // The outbound session is established lazily, on this device's
        // first actual send to them (see `deliver`) — not here. Olm has
        // exactly one session per pair; eagerly creating an outbound
        // session on both sides at pairing time would create two
        // independent, mismatched sessions instead of one shared one.
        self.store
            .set_contact_pending_otk(contact_id, Some(&their_one_time_key))?;

        // Bound to a `let` first, not matched directly in the `if let`
        // scrutinee: temporaries in an `if let` condition live for the
        // whole block in Rust, which would hold this `MutexGuard` across
        // the `.await` below — not just non-idiomatic, a compile error
        // (`MutexGuard` isn't `Send`).
        let my_addr = self.server_addr.lock().unwrap().clone();
        if let Some(my_addr) = my_addr {
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

    fn get_contact(&self, contact_id: i64) -> Result<ContactRecord> {
        self.store
            .get_contact(contact_id)?
            .ok_or(CoreError::UnknownContact(contact_id))
    }

    /// Encrypts `payload` for `contact_id` and delivers it via whichever
    /// path applies (`docs/PRD.md` Flow 3): that contact's Server mailbox
    /// if they have one on file, otherwise a direct P2P attempt with a
    /// ~15-20s timeout (Local-to-local). Returns the message id, lamport,
    /// sent_at, and resulting status — used by both `send` (wraps the
    /// caller's plaintext as `Chat`) and `send_receipt` (wraps as
    /// `Receipt`), which differ only in whether/how they persist the
    /// result afterward.
    async fn deliver(
        &self,
        contact_id: i64,
        payload: &PlaintextPayload,
    ) -> Result<([u8; 16], u64, u64, MessageStatus)> {
        let _guard = self.send_lock.lock().await;
        let contact = self.get_contact(contact_id)?;

        let mut session = match self.store.load_session_pickle(contact_id)? {
            Some(pickle) => MySession::from_pickle(&pickle)?,
            None => {
                let their_otk = contact.pending_otk.ok_or(CoreError::NoSessionAvailable)?;
                let their_curve =
                    vodozemac::Curve25519PublicKey::from_bytes(contact.curve25519_key);
                let their_otk = vodozemac::Curve25519PublicKey::from_bytes(their_otk);
                let session = {
                    let account = self.account.lock().unwrap();
                    account.create_outbound_session(their_curve, their_otk)?
                };
                self.store.set_contact_pending_otk(contact_id, None)?;
                session
            }
        };

        let plaintext_bytes = payload.serialize();
        let (olm_type, ciphertext) = session.encrypt(&plaintext_bytes)?;

        let mut msg_id = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut msg_id);
        let sent_at = now_millis();
        let lamport = self.store.tick_lamport()?;

        let status = if let Some(server_addr_bytes) = &contact.server_addr {
            let their_server_addr: EndpointAddr = bincode::deserialize(server_addr_bytes)
                .map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
            let pair_secret = contact.pair_secret.ok_or(CoreError::NoRouteToContact)?;
            let n = self.store.increment_and_get_next_write_n(contact_id)?;
            let auth = derive_auth(&pair_secret, n)?;

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
            MessageStatus::Sent
        } else {
            let their_transport_key = contact.transport_key.ok_or(CoreError::NoRouteToContact)?;
            let endpoint_id = EndpointId::from_bytes(&their_transport_key)
                .map_err(|e| CoreError::CorruptBackup(e.to_string()))?;
            let peer_addr = EndpointAddr::from(endpoint_id);

            // `n` (the write-auth sequence number) has no meaning here —
            // there's no shared-server slot scheme to authenticate against
            // for a direct connection between two already-paired contacts.
            let envelope = Envelope::new(olm_type, ciphertext, 0, lamport, sent_at, msg_id);
            let envelope_bytes = envelope.to_padded_bytes()?;

            match self.try_direct(peer_addr, envelope_bytes.clone()).await {
                Ok(()) => MessageStatus::Delivered,
                Err(direct_err) => {
                    // M7 (`docs/PRD.md` Flow 3's third case): the recipient
                    // isn't reachable right now — if this device has its own
                    // Server, hand delivery off to it instead of failing
                    // outright. Its eventual success (or exhaustion) reaches
                    // this message later via the ordinary receipt path (a
                    // successful retry looks just like any other Local
                    // delivery to its recipient) or `sync`'s
                    // `PollFailedDeliveries`, not anything returned here.
                    let my_addr = self.server_addr.lock().unwrap().clone();
                    let queued = match my_addr {
                        Some(my_addr) => self
                            .schedule_retry(my_addr, msg_id, their_transport_key, envelope_bytes)
                            .await
                            .is_ok(),
                        None => false,
                    };
                    if queued {
                        MessageStatus::Sent
                    } else {
                        return Err(direct_err);
                    }
                }
            }
        };

        self.store
            .save_session_pickle(contact_id, &session.pickle())?;
        Ok((msg_id, lamport, sent_at, status))
    }

    /// One direct-delivery attempt, bounded by `DIRECT_DELIVERY_TIMEOUT`.
    /// Split out of `deliver` so a failure can be caught and weighed
    /// against the M7 retry-queue fallback instead of always propagating.
    async fn try_direct(&self, peer_addr: EndpointAddr, envelope_bytes: Vec<u8>) -> Result<()> {
        tokio::time::timeout(
            DIRECT_DELIVERY_TIMEOUT,
            self.node_client.call_direct(peer_addr, envelope_bytes),
        )
        .await
        .map_err(|_| CoreError::DirectDeliveryTimedOut)??;
        Ok(())
    }

    /// M7: asks this device's own Server (`my_addr`) to take over
    /// retrying delivery of `envelope` to `recipient_transport_key`, since
    /// a direct attempt already failed.
    async fn schedule_retry(
        &self,
        my_addr: EndpointAddr,
        msg_id: [u8; 16],
        recipient_transport_key: [u8; 32],
        envelope: Vec<u8>,
    ) -> Result<()> {
        let response = self
            .node_client
            .call(
                my_addr,
                &NodeRequest::ScheduleRetry {
                    mailbox_key: self.identity.mailbox_key,
                    msg_id,
                    recipient_transport_key,
                    envelope,
                },
            )
            .await?;
        expect_ok(response)
    }

    async fn send(&self, contact_id: i64, plaintext: &[u8]) -> Result<()> {
        let payload = PlaintextPayload::Chat(plaintext.to_vec());
        let (msg_id, lamport, sent_at, status) = self.deliver(contact_id, &payload).await?;
        self.store.insert_message(
            contact_id,
            NewMessage {
                msg_id,
                direction: Direction::Outgoing,
                lamport,
                sent_at,
                plaintext,
                status: Some(status),
            },
        )?;
        Ok(())
    }

    /// Best-effort: a receipt failing to send must never fail whatever
    /// triggered it (receiving the original message already succeeded on
    /// its own). Errors are simply dropped.
    async fn send_receipt(&self, contact_id: i64, for_msg_id: [u8; 16]) {
        let payload = PlaintextPayload::Receipt { msg_id: for_msg_id };
        let _ = self.deliver(contact_id, &payload).await;
    }

    /// Associated function rather than a `&self` method (see the module
    /// doc comment): needs an owned `Arc<Self>` on hand so `Chat` receipt
    /// sending (via `handle_decrypted`) can spawn it as a background task.
    async fn sync(shared: &Arc<Self>) -> Result<usize> {
        let my_addr = shared
            .server_addr
            .lock()
            .unwrap()
            .clone()
            .ok_or(CoreError::NoOwnServer)?;
        let response = shared
            .node_client
            .call(
                my_addr,
                &NodeRequest::Fetch {
                    mailbox_key: shared.identity.mailbox_key,
                },
            )
            .await?;
        let NodeResponse::Blobs(blobs) = response else {
            return Err(unexpected_response(response));
        };

        let watermark = shared.store.get_last_synced_blob_id()?;
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
            if let Attribution::Chat = Self::attribute_and_store(shared, &envelope)? {
                processed += 1;
            }
            acked_ids.push(blob.id);
        }

        if !acked_ids.is_empty() {
            let server_addr = shared.server_addr.lock().unwrap().clone().unwrap();
            let response = shared
                .node_client
                .call(
                    server_addr,
                    &NodeRequest::Ack {
                        mailbox_key: shared.identity.mailbox_key,
                        ids: acked_ids,
                    },
                )
                .await?;
            expect_ok(response)?;
        }
        shared.store.set_last_synced_blob_id(max_id_seen)?;

        // M7: pick up any of this device's own queued sends whose retry
        // schedule exhausted since the last sync, and mark them Failed —
        // `docs/PRD.md` Flow 3's "the message stays in the thread marked
        // 'failed to deliver'". Reuses `my_addr` from above; a failure
        // here shouldn't fail the sync that already succeeded, so it's
        // propagated the same way any other own-server call in this
        // function would be (a transient own-server outage just means
        // trying again on the next sync).
        let server_addr = shared.server_addr.lock().unwrap().clone().unwrap();
        let failed_response = shared
            .node_client
            .call(
                server_addr,
                &NodeRequest::PollFailedDeliveries {
                    mailbox_key: shared.identity.mailbox_key,
                },
            )
            .await?;
        let NodeResponse::FailedDeliveries(failed_msg_ids) = failed_response else {
            return Err(unexpected_response(failed_response));
        };
        for msg_id in failed_msg_ids {
            shared
                .store
                .set_message_status_by_msg_id(&msg_id, MessageStatus::Failed)?;
        }

        Ok(processed)
    }

    /// Tries each known contact's session (or, if none is established yet,
    /// tries establishing an inbound one from this being their first
    /// message) until one successfully decrypts the envelope, then hands
    /// off to `handle_decrypted`. Used by both `sync` (Server mailbox) and
    /// `DirectHandler` (Local delivery) — the two paths share everything
    /// past "here's a decrypted envelope".
    fn attribute_and_store(shared: &Arc<Self>, envelope: &Envelope) -> Result<Attribution> {
        let contacts = shared.store.list_contacts()?;
        for contact in contacts {
            if let Some(pickle) = shared.store.load_session_pickle(contact.id)? {
                let mut session = MySession::from_pickle(&pickle)?;
                if let Ok(plaintext) = session.decrypt(envelope.olm_type, &envelope.ciphertext) {
                    shared
                        .store
                        .save_session_pickle(contact.id, &session.pickle())?;
                    return Self::handle_decrypted(shared, contact.id, envelope, plaintext);
                }
            } else {
                let their_curve =
                    vodozemac::Curve25519PublicKey::from_bytes(contact.curve25519_key);
                let accepted = {
                    let mut account = shared.account.lock().unwrap();
                    account
                        .accept_incoming(their_curve, envelope.olm_type, &envelope.ciphertext)
                        .ok()
                        .map(|(session, plaintext)| (account.pickle(), session, plaintext))
                };
                if let Some((account_pickle, session, plaintext)) = accepted {
                    shared.store.save_account_pickle(&account_pickle)?;
                    shared
                        .store
                        .save_session_pickle(contact.id, &session.pickle())?;
                    return Self::handle_decrypted(shared, contact.id, envelope, plaintext);
                }
            }
        }
        Ok(Attribution::Unattributed)
    }

    /// A successfully-decrypted envelope, attributed to `contact_id`.
    /// Branches on the plaintext wrapper: a real chat message gets
    /// persisted and, best-effort, acknowledged back to its sender with a
    /// `Receipt` — spawned rather than awaited inline, so a slow or
    /// unreachable sender doesn't hold up whatever's calling this (an
    /// incoming connection waiting on its ack, or `sync`). A `Receipt`
    /// updates the original outgoing message's status instead of becoming
    /// a message itself. A payload that decrypts but doesn't parse as
    /// either (a version mismatch, most likely — see
    /// `PlaintextPayload::deserialize`'s docs) is silently skipped, same
    /// as an unattributable envelope.
    fn handle_decrypted(
        shared: &Arc<Self>,
        contact_id: i64,
        envelope: &Envelope,
        plaintext: Vec<u8>,
    ) -> Result<Attribution> {
        match PlaintextPayload::deserialize(&plaintext) {
            Some(PlaintextPayload::Chat(text)) => {
                let lamport = shared.store.observe_lamport(envelope.lamport)?;
                shared.store.insert_message(
                    contact_id,
                    NewMessage {
                        msg_id: envelope.msg_id,
                        direction: Direction::Incoming,
                        lamport,
                        sent_at: envelope.sent_at,
                        plaintext: &text,
                        status: None,
                    },
                )?;

                let owned = Arc::clone(shared);
                let msg_id = envelope.msg_id;
                tokio::spawn(async move {
                    owned.send_receipt(contact_id, msg_id).await;
                });

                Ok(Attribution::Chat)
            }
            Some(PlaintextPayload::Receipt { msg_id }) => {
                shared
                    .store
                    .set_message_status(contact_id, &msg_id, MessageStatus::Delivered)?;
                Ok(Attribution::Receipt)
            }
            None => Ok(Attribution::Unattributed), // malformed — skip, don't fail the caller
        }
    }

    async fn push_backup(&self) -> Result<()> {
        let my_addr = self
            .server_addr
            .lock()
            .unwrap()
            .clone()
            .ok_or(CoreError::NoOwnServer)?;
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

    fn export_backup(&self) -> Result<Vec<u8>> {
        let bundle = backup::assemble(&self.store)?;
        let plaintext = backup::serialize(&bundle);
        Ok(pm_crypto::encrypt_backup(
            &self.identity.backup_key,
            &plaintext,
        ))
    }
}

/// What decrypting and handling an envelope turned out to be — used so
/// `sync` can tell a real chat message (worth counting/returning) apart
/// from a receipt or an unattributable envelope (neither is).
enum Attribution {
    Chat,
    Receipt,
    Unattributed,
}

/// Accepts direct, Local-to-local deliveries on [`DIRECT_ALPN`] — the
/// counterpart to `pm-node`'s `MailboxHandler`, but running inside the app
/// itself rather than a separate server process. Holds only a `Weak`
/// reference; see the module doc comment for why.
#[derive(Debug, Clone)]
struct DirectHandler {
    shared: Weak<ClientShared>,
}

impl ProtocolHandler for DirectHandler {
    async fn accept(&self, connection: Connection) -> std::result::Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let envelope_bytes = recv
            .read_to_end(pm_proto::MAX_MESSAGE_SIZE)
            .await
            .map_err(std::io::Error::other)?;

        let ack = match self.shared.upgrade() {
            None => DirectAck::Error("this device is no longer reachable".to_string()),
            Some(shared) => match Envelope::from_padded_bytes(&envelope_bytes) {
                Err(e) => DirectAck::Error(format!("malformed envelope: {e}")),
                Ok(envelope) => match ClientShared::attribute_and_store(&shared, &envelope) {
                    Ok(Attribution::Chat | Attribution::Receipt) => DirectAck::Ok,
                    Ok(Attribution::Unattributed) => DirectAck::Error(
                        "could not attribute this message to any known contact".to_string(),
                    ),
                    Err(e) => DirectAck::Error(e.to_string()),
                },
            },
        };

        let ack_bytes = bincode::serialize(&ack).expect("DirectAck serialization cannot fail");
        send.write_all(&ack_bytes)
            .await
            .map_err(std::io::Error::other)?;
        send.finish()?;

        connection.closed().await;
        Ok(())
    }
}

/// Shared by `open` and `import_backup`: decodes this device's own Server
/// mailbox address from the store, if one is set.
fn load_own_server_addr(store: &Store) -> Result<Option<EndpointAddr>> {
    store
        .get_own_server_addr()?
        .map(|bytes| bincode::deserialize(&bytes))
        .transpose()
        .map_err(|e: bincode::Error| CoreError::CorruptBackup(e.to_string()))
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
