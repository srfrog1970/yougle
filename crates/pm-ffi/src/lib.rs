//! `pm-ffi`: uniffi interface definitions exposing `pm-core`'s `Client` to
//! the React Native app via `uniffi-bindgen-react-native` (Turbo Modules).
//!
//! `pm_core::Client` is its own internally-synchronized, cheaply-`Clone`
//! handle (see its module doc comment) — its background Local-delivery
//! accept loop needs that regardless of FFI, so there's no need for this
//! layer to add its *own* outer lock on top the way it did through M5;
//! every method here just calls straight into `Client`'s own `&self`
//! methods.
//!
//! Server mailbox addresses cross this boundary as plain strings (see
//! `pm_transport::encode_endpoint_addr`/`decode_endpoint_addr`) rather than
//! raw bytes — they're meant to be pasted, displayed, and put in a QR code,
//! so every method that takes or returns one (`restore`, `add_contact`,
//! `own_server_addr`, `set_own_server_addr`) does the string conversion
//! itself; the RN layer never handles raw address bytes.

use std::path::PathBuf;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("{0}")]
    Failed(String),
}

impl From<pm_core::CoreError> for FfiError {
    fn from(e: pm_core::CoreError) -> Self {
        FfiError::Failed(e.to_string())
    }
}

impl From<pm_crypto::CryptoError> for FfiError {
    fn from(e: pm_crypto::CryptoError) -> Self {
        FfiError::Failed(e.to_string())
    }
}

impl From<pm_transport::TransportError> for FfiError {
    fn from(e: pm_transport::TransportError) -> Self {
        FfiError::Failed(e.to_string())
    }
}

fn to_32(bytes: Vec<u8>, field: &str) -> Result<[u8; 32], FfiError> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        FfiError::Failed(format!("{field} must be exactly 32 bytes, got {}", v.len()))
    })
}

fn parse_addr(s: &str) -> Result<iroh::EndpointAddr, FfiError> {
    pm_transport::decode_endpoint_addr(s).map_err(Into::into)
}

fn format_addr(addr: &iroh::EndpointAddr) -> Result<String, FfiError> {
    pm_transport::encode_endpoint_addr(addr).map_err(Into::into)
}

/// uniffi's async bridging polls exported futures from whatever thread the
/// foreign (Kotlin/Swift/JS) side drives them from — not necessarily one
/// with a Tokio runtime entered. `iroh` (and anything else in `pm-core`
/// that spawns tasks or uses Tokio's reactor) needs one, or every call
/// fails at runtime with "there is no reactor running, must be called from
/// the context of a Tokio 1.x runtime" — a failure invisible to `cargo
/// build`/`cargo test`, since `#[tokio::test]` always provides a runtime.
/// `async_compat::Compat` (re-exported by uniffi's own `tokio` feature)
/// fixes this by entering a lazily-created background Tokio runtime on
/// every poll, regardless of what's actually driving the outer future.
async fn compat<F: std::future::Future>(fut: F) -> F::Output {
    uniffi::deps::async_compat::Compat::new(fut).await
}

/// Delivery status for an outgoing message — see `pm_store::MessageStatus`.
/// `None` for incoming messages, where status isn't a concept that applies.
#[derive(uniffi::Enum)]
pub enum FfiMessageStatus {
    Sent,
    Delivered,
    Failed,
}

impl From<pm_store::MessageStatus> for FfiMessageStatus {
    fn from(s: pm_store::MessageStatus) -> Self {
        match s {
            pm_store::MessageStatus::Sent => FfiMessageStatus::Sent,
            pm_store::MessageStatus::Delivered => FfiMessageStatus::Delivered,
            pm_store::MessageStatus::Failed => FfiMessageStatus::Failed,
        }
    }
}

#[derive(uniffi::Record)]
pub struct FfiMessage {
    pub msg_id: Vec<u8>,
    pub outgoing: bool,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: Vec<u8>,
    pub status: Option<FfiMessageStatus>,
}

impl From<pm_store::StoredMessage> for FfiMessage {
    fn from(m: pm_store::StoredMessage) -> Self {
        Self {
            msg_id: m.msg_id.to_vec(),
            outgoing: matches!(m.direction, pm_store::Direction::Outgoing),
            lamport: m.lamport,
            sent_at: m.sent_at,
            plaintext: m.plaintext,
            status: m.status.map(Into::into),
        }
    }
}

#[derive(uniffi::Record)]
pub struct FfiContact {
    pub id: i64,
    pub identity_key: Vec<u8>,
    pub curve25519_key: Vec<u8>,
    pub display_name: Option<String>,
    /// Whether this contact has a Server mailbox on file. Sending works
    /// either way as of M6 (a contact with no Server mailbox gets a direct
    /// Local delivery attempt instead) — this is informational (e.g. for
    /// showing "Local only" in a contact list), not a capability gate.
    pub has_server: bool,
}

impl From<pm_store::ContactRecord> for FfiContact {
    fn from(c: pm_store::ContactRecord) -> Self {
        Self {
            id: c.id,
            identity_key: c.identity_key.to_vec(),
            curve25519_key: c.curve25519_key.to_vec(),
            display_name: c.display_name,
            has_server: c.server_addr.is_some(),
        }
    }
}

/// One device's shareable pairing data — see `pm_core::PairingPayload`.
/// Byte fields, not strings: this is meant to be packed as a whole (by the
/// RN layer, into a QR code or paste code), not read field-by-field.
#[derive(uniffi::Record)]
pub struct FfiPairingPayload {
    pub identity_key: Vec<u8>,
    pub curve25519_key: Vec<u8>,
    pub transport_key: Vec<u8>,
    pub one_time_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub server_addr: Option<Vec<u8>>,
}

impl From<pm_core::PairingPayload> for FfiPairingPayload {
    fn from(p: pm_core::PairingPayload) -> Self {
        Self {
            identity_key: p.identity_key.to_vec(),
            curve25519_key: p.curve25519_key.to_vec(),
            transport_key: p.transport_key.to_vec(),
            one_time_key: p.one_time_key.to_vec(),
            nonce: p.nonce.to_vec(),
            server_addr: p.server_addr,
        }
    }
}

impl TryFrom<FfiPairingPayload> for pm_core::PairingPayload {
    type Error = FfiError;

    fn try_from(p: FfiPairingPayload) -> Result<Self, FfiError> {
        Ok(Self {
            identity_key: to_32(p.identity_key, "identity_key")?,
            curve25519_key: to_32(p.curve25519_key, "curve25519_key")?,
            transport_key: to_32(p.transport_key, "transport_key")?,
            one_time_key: to_32(p.one_time_key, "one_time_key")?,
            nonce: to_32(p.nonce, "nonce")?,
            server_addr: p.server_addr,
        })
    }
}

/// The FFI-facing handle to a `pm-core` client. One instance per identity
/// per app session.
#[derive(uniffi::Object)]
pub struct FfiClient {
    inner: pm_core::Client,
}

#[uniffi::export]
impl FfiClient {
    /// Opens (creating if needed) a client backed by the encrypted store at
    /// `store_path`, for the identity the 24-word `seed_phrase` derives.
    /// This device's own Server mailbox (if any) is remembered from a
    /// previous `set_own_server_addr` call, not supplied here — see
    /// `own_server_addr`.
    #[uniffi::constructor]
    pub async fn open(seed_phrase: String, store_path: String) -> Result<Self, FfiError> {
        compat(async move {
            let seed = pm_crypto::Seed::from_phrase(&seed_phrase)?;
            let inner = pm_core::Client::open(&seed, &PathBuf::from(store_path)).await?;
            Ok(Self { inner })
        })
        .await
    }

    /// Restores identity, contacts, and message history from a backup
    /// previously pushed to `server_addr` — see `pm-core`'s docs for why
    /// this address must be supplied directly rather than looked up.
    #[uniffi::constructor]
    pub async fn restore(
        seed_phrase: String,
        store_path: String,
        server_addr: String,
    ) -> Result<Self, FfiError> {
        compat(async move {
            let seed = pm_crypto::Seed::from_phrase(&seed_phrase)?;
            let addr = parse_addr(&server_addr)?;
            let inner = pm_core::Client::restore(&seed, &PathBuf::from(store_path), addr).await?;
            Ok(Self { inner })
        })
        .await
    }

    /// Restores identity, contacts, and message history from a manually
    /// supplied encrypted backup file's bytes (as produced by
    /// `export_backup`), rather than fetching one from a Server.
    #[uniffi::constructor]
    pub async fn import_backup(
        seed_phrase: String,
        store_path: String,
        backup_bytes: Vec<u8>,
    ) -> Result<Self, FfiError> {
        compat(async move {
            let seed = pm_crypto::Seed::from_phrase(&seed_phrase)?;
            let inner =
                pm_core::Client::import_backup(&seed, &PathBuf::from(store_path), &backup_bytes)
                    .await?;
            Ok(Self { inner })
        })
        .await
    }

    pub async fn identity_key(&self) -> Vec<u8> {
        compat(async { self.inner.identity_key().to_vec() }).await
    }

    pub async fn curve25519_key(&self) -> Vec<u8> {
        compat(async { self.inner.curve25519_key().to_vec() }).await
    }

    /// This device's own mailbox key — one of the two values a self-hosted
    /// `pm-node` needs (`PM_NODE_MAILBOX_KEY`) to run as this identity's own
    /// Server. See `deploy/README.md`.
    pub async fn mailbox_key(&self) -> Vec<u8> {
        compat(async { self.inner.mailbox_key().to_vec() }).await
    }

    /// This device's own self-hosted-node transport identity — the other
    /// value a self-hosted `pm-node` needs (`PM_NODE_TRANSPORT_KEY`).
    pub async fn server_transport_key(&self) -> Vec<u8> {
        compat(async { self.inner.server_transport_key().to_vec() }).await
    }

    /// Generates a one-time key for a pairing partner (stand-in for what a
    /// real QR payload would carry — see `pm-core`'s docs).
    pub async fn generate_one_time_key(&self) -> Result<Vec<u8>, FfiError> {
        compat(async { Ok(self.inner.generate_one_time_key()?.to_vec()) }).await
    }

    pub async fn list_contacts(&self) -> Result<Vec<FfiContact>, FfiError> {
        compat(async {
            Ok(self
                .inner
                .list_contacts()?
                .into_iter()
                .map(Into::into)
                .collect())
        })
        .await
    }

    /// This device's own Server mailbox address, if it has configured one.
    pub async fn own_server_addr(&self) -> Result<Option<String>, FfiError> {
        compat(async {
            self.inner
                .own_server_addr()
                .map(|addr| format_addr(&addr))
                .transpose()
        })
        .await
    }

    /// Configures this device's own Server mailbox from a pasted/scanned
    /// address string.
    pub async fn set_own_server_addr(&self, addr: String) -> Result<(), FfiError> {
        compat(async move {
            let addr = parse_addr(&addr)?;
            self.inner.set_own_server_addr(addr)?;
            Ok(())
        })
        .await
    }

    pub async fn clear_own_server_addr(&self) -> Result<(), FfiError> {
        compat(async {
            self.inner.clear_own_server_addr()?;
            Ok(())
        })
        .await
    }

    /// Produces this device's shareable pairing data. Hold onto the
    /// returned `nonce` — it's needed again, unchanged, when this same
    /// pairing attempt is completed via `add_contact_from_payload`.
    pub async fn pairing_payload(&self) -> Result<FfiPairingPayload, FfiError> {
        compat(async { Ok(self.inner.pairing_payload()?.into()) }).await
    }

    /// Completes a pairing exchange: `their` is the partner's
    /// `pairing_payload()`, and `my_nonce` is the `nonce` from *this*
    /// device's own `pairing_payload()` call for the same attempt.
    pub async fn add_contact_from_payload(
        &self,
        their: FfiPairingPayload,
        my_nonce: Vec<u8>,
        display_name: Option<String>,
    ) -> Result<i64, FfiError> {
        compat(async move {
            let their: pm_core::PairingPayload = their.try_into()?;
            let my_nonce = to_32(my_nonce, "my_nonce")?;

            Ok(self
                .inner
                .add_contact_from_payload(their, my_nonce, display_name.as_deref())
                .await?)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_contact(
        &self,
        their_identity_key: Vec<u8>,
        their_curve25519_key: Vec<u8>,
        their_one_time_key: Vec<u8>,
        their_transport_key: Vec<u8>,
        display_name: Option<String>,
        their_server_addr: Option<String>,
        pair_secret: Vec<u8>,
    ) -> Result<i64, FfiError> {
        compat(async move {
            let their_identity_key = to_32(their_identity_key, "their_identity_key")?;
            let their_curve25519_key = to_32(their_curve25519_key, "their_curve25519_key")?;
            let their_one_time_key = to_32(their_one_time_key, "their_one_time_key")?;
            let their_transport_key = to_32(their_transport_key, "their_transport_key")?;
            let pair_secret = to_32(pair_secret, "pair_secret")?;
            let their_server_addr = their_server_addr.map(|s| parse_addr(&s)).transpose()?;

            let id = self
                .inner
                .add_contact(
                    their_identity_key,
                    their_curve25519_key,
                    their_one_time_key,
                    their_transport_key,
                    display_name.as_deref(),
                    their_server_addr,
                    pair_secret,
                )
                .await?;
            Ok(id)
        })
        .await
    }

    /// Encrypts and delivers a message — via the recipient's Server
    /// mailbox if they have one, otherwise a direct Local P2P attempt (see
    /// `pm-core::Client::send`'s docs for the ~15-20s timeout and what a
    /// failure means for the caller).
    pub async fn send(&self, contact_id: i64, plaintext: Vec<u8>) -> Result<(), FfiError> {
        compat(async move {
            self.inner.send(contact_id, &plaintext).await?;
            Ok(())
        })
        .await
    }

    /// Fetches and processes new messages from this client's own Server.
    /// Returns how many new *chat* messages were processed (delivery
    /// receipts update existing messages' status rather than counting).
    pub async fn sync(&self) -> Result<u32, FfiError> {
        compat(async { Ok(self.inner.sync().await? as u32) }).await
    }

    pub async fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<FfiMessage>, FfiError> {
        compat(async {
            Ok(self
                .inner
                .messages_for_contact(contact_id)?
                .into_iter()
                .map(Into::into)
                .collect())
        })
        .await
    }

    pub async fn push_backup(&self) -> Result<(), FfiError> {
        compat(async {
            self.inner.push_backup().await?;
            Ok(())
        })
        .await
    }

    /// Assembles and encrypts the same backup bundle as `push_backup`, but
    /// returns the ciphertext directly instead of pushing it to a Server —
    /// works with no Server mailbox configured at all.
    pub async fn export_backup(&self) -> Result<Vec<u8>, FfiError> {
        compat(async { Ok(self.inner.export_backup()?) }).await
    }
}

/// Generates a fresh 24-word BIP39 recovery phrase. No `Client` exists yet
/// at onboarding time, so this is a free function rather than a method.
#[uniffi::export]
pub fn generate_seed_phrase() -> String {
    pm_crypto::Seed::generate().1.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn to_32_rejects_wrong_length() {
        assert!(to_32(vec![0u8; 31], "x").is_err());
        assert!(to_32(vec![0u8; 32], "x").is_ok());
    }

    #[test]
    fn generate_seed_phrase_returns_24_words() {
        let phrase = generate_seed_phrase();
        assert_eq!(phrase.split_whitespace().count(), 24);
    }
}
