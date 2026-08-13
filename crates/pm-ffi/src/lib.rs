//! `pm-ffi`: uniffi interface definitions exposing `pm-core`'s `Client` to
//! the React Native app via `uniffi-bindgen-react-native` (Turbo Modules).
//!
//! `pm_core::Client`'s methods take `&mut self`, but uniffi objects are
//! always accessed through `&self` (foreign callers hold a shared
//! reference) — so this wraps `Client` in a `tokio::sync::Mutex` and every
//! exported method locks it, even the read-only ones, for one consistent
//! access pattern.

use std::path::PathBuf;

use tokio::sync::Mutex;

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

fn to_32(bytes: Vec<u8>, field: &str) -> Result<[u8; 32], FfiError> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        FfiError::Failed(format!("{field} must be exactly 32 bytes, got {}", v.len()))
    })
}

fn deserialize_addr(bytes: &[u8]) -> Result<iroh::EndpointAddr, FfiError> {
    bincode::deserialize(bytes)
        .map_err(|e| FfiError::Failed(format!("invalid server address: {e}")))
}

#[derive(uniffi::Record)]
pub struct FfiMessage {
    pub msg_id: Vec<u8>,
    pub outgoing: bool,
    pub lamport: u64,
    pub sent_at: u64,
    pub plaintext: Vec<u8>,
}

impl From<pm_store::StoredMessage> for FfiMessage {
    fn from(m: pm_store::StoredMessage) -> Self {
        Self {
            msg_id: m.msg_id.to_vec(),
            outgoing: matches!(m.direction, pm_store::Direction::Outgoing),
            lamport: m.lamport,
            sent_at: m.sent_at,
            plaintext: m.plaintext,
        }
    }
}

/// The FFI-facing handle to a `pm-core` client. One instance per identity
/// per app session.
#[derive(uniffi::Object)]
pub struct FfiClient {
    inner: Mutex<pm_core::Client>,
}

#[uniffi::export]
impl FfiClient {
    /// Opens (creating if needed) a client backed by the encrypted store at
    /// `store_path`, for the identity the 24-word `seed_phrase` derives.
    /// `server_addr` is this device's own Server mailbox address (bincode-
    /// serialized `iroh::EndpointAddr` bytes, as produced by
    /// `own_server_addr()` elsewhere on this same client), or `None` for a
    /// Local-only client.
    #[uniffi::constructor]
    pub async fn open(
        seed_phrase: String,
        store_path: String,
        server_addr: Option<Vec<u8>>,
    ) -> Result<Self, FfiError> {
        let seed = pm_crypto::Seed::from_phrase(&seed_phrase)?;
        let addr = server_addr.map(|b| deserialize_addr(&b)).transpose()?;
        let client = pm_core::Client::open(&seed, &PathBuf::from(store_path), addr).await?;
        Ok(Self {
            inner: Mutex::new(client),
        })
    }

    /// Restores identity, contacts, and message history from a backup
    /// previously pushed to `server_addr` — see `pm-core`'s docs for why
    /// this address must be supplied directly rather than looked up.
    #[uniffi::constructor]
    pub async fn restore(
        seed_phrase: String,
        store_path: String,
        server_addr: Vec<u8>,
    ) -> Result<Self, FfiError> {
        let seed = pm_crypto::Seed::from_phrase(&seed_phrase)?;
        let addr = deserialize_addr(&server_addr)?;
        let client = pm_core::Client::restore(&seed, &PathBuf::from(store_path), addr).await?;
        Ok(Self {
            inner: Mutex::new(client),
        })
    }

    pub async fn identity_key(&self) -> Vec<u8> {
        self.inner.lock().await.identity_key().to_vec()
    }

    pub async fn curve25519_key(&self) -> Vec<u8> {
        self.inner.lock().await.curve25519_key().to_vec()
    }

    /// Generates a one-time key for a pairing partner (stand-in for what a
    /// real QR payload would carry — see `pm-core`'s docs).
    pub async fn generate_one_time_key(&self) -> Result<Vec<u8>, FfiError> {
        let mut inner = self.inner.lock().await;
        Ok(inner.generate_one_time_key()?.to_vec())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_contact(
        &self,
        their_identity_key: Vec<u8>,
        their_curve25519_key: Vec<u8>,
        their_one_time_key: Vec<u8>,
        display_name: Option<String>,
        their_server_addr: Option<Vec<u8>>,
        pair_secret: Vec<u8>,
    ) -> Result<i64, FfiError> {
        let their_identity_key = to_32(their_identity_key, "their_identity_key")?;
        let their_curve25519_key = to_32(their_curve25519_key, "their_curve25519_key")?;
        let their_one_time_key = to_32(their_one_time_key, "their_one_time_key")?;
        let pair_secret = to_32(pair_secret, "pair_secret")?;
        let their_server_addr = their_server_addr
            .map(|b| deserialize_addr(&b))
            .transpose()?;

        let mut inner = self.inner.lock().await;
        let id = inner
            .add_contact(
                their_identity_key,
                their_curve25519_key,
                their_one_time_key,
                display_name.as_deref(),
                their_server_addr,
                pair_secret,
            )
            .await?;
        Ok(id)
    }

    pub async fn send(&self, contact_id: i64, plaintext: Vec<u8>) -> Result<(), FfiError> {
        let mut inner = self.inner.lock().await;
        inner.send(contact_id, &plaintext).await?;
        Ok(())
    }

    /// Fetches and processes new messages from this client's own Server.
    /// Returns how many were processed.
    pub async fn sync(&self) -> Result<u32, FfiError> {
        let mut inner = self.inner.lock().await;
        Ok(inner.sync().await? as u32)
    }

    pub async fn messages_for_contact(&self, contact_id: i64) -> Result<Vec<FfiMessage>, FfiError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .messages_for_contact(contact_id)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn push_backup(&self) -> Result<(), FfiError> {
        let inner = self.inner.lock().await;
        inner.push_backup().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn to_32_rejects_wrong_length() {
        assert!(to_32(vec![0u8; 31], "x").is_err());
        assert!(to_32(vec![0u8; 32], "x").is_ok());
    }
}
