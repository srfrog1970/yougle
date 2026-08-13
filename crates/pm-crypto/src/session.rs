//! Wraps vodozemac's Olm `Account` and `Session` and adapts message framing
//! to/from the `(olm_type, ciphertext)` shape `pm_proto::Envelope` stores.
//!
//! These keys are randomly generated per account, not derived from the seed
//! — see `identity.rs` for why that's the correct split, not an omission.

use std::collections::HashMap;

use vodozemac::olm::{Account, OlmMessage, PreKeyMessage, Session, SessionConfig};
use vodozemac::{Curve25519PublicKey, KeyId};

use crate::error::{CryptoError, Result};

pub struct MyAccount {
    inner: Account,
}

impl Default for MyAccount {
    fn default() -> Self {
        Self::new()
    }
}

impl MyAccount {
    pub fn new() -> Self {
        Self {
            inner: Account::new(),
        }
    }

    pub fn curve25519_key(&self) -> Curve25519PublicKey {
        self.inner.curve25519_key()
    }

    /// Serializes this account's state (identity + one-time keys) for
    /// storage, so it can be restored with [`MyAccount::from_pickle`] after
    /// an app restart. Uses JSON rather than bincode: vodozemac's pickle
    /// types require a self-describing serde format.
    pub fn pickle(&self) -> Vec<u8> {
        serde_json::to_vec(&self.inner.pickle()).expect("AccountPickle serialization cannot fail")
    }

    /// Restores an account previously serialized with [`MyAccount::pickle`].
    pub fn from_pickle(bytes: &[u8]) -> Result<Self> {
        let pickle = serde_json::from_slice(bytes)
            .map_err(|e| CryptoError::MalformedMessage(e.to_string()))?;
        Ok(Self {
            inner: Account::from_pickle(pickle),
        })
    }

    /// Generates `count` one-time keys and returns their public parts, ready
    /// to publish (e.g., embedded in a pairing QR).
    pub fn generate_one_time_keys(&mut self, count: usize) -> HashMap<KeyId, Curve25519PublicKey> {
        self.inner.generate_one_time_keys(count);
        self.inner.one_time_keys()
    }

    /// Starts a session to a peer, consuming one of their published one-time
    /// keys. The first message sent over the returned session will be a
    /// `PreKey` message that the peer uses to establish their inbound side.
    pub fn create_outbound_session(
        &self,
        their_identity_key: Curve25519PublicKey,
        their_one_time_key: Curve25519PublicKey,
    ) -> Result<MySession> {
        let session = self
            .inner
            .create_outbound_session(
                SessionConfig::version_1(),
                their_identity_key,
                their_one_time_key,
            )
            .map_err(|e| CryptoError::SessionCreation(e.to_string()))?;
        Ok(MySession { inner: session })
    }

    /// Establishes the inbound side of a session from the first
    /// (`PreKey`-framed) message received from a peer, returning the session
    /// and that first message's plaintext together, since vodozemac decrypts
    /// it as part of establishing the session.
    pub fn accept_incoming(
        &mut self,
        their_identity_key: Curve25519PublicKey,
        olm_type: u8,
        ciphertext: &[u8],
    ) -> Result<(MySession, Vec<u8>)> {
        let message = OlmMessage::from_parts(olm_type as usize, ciphertext)
            .map_err(|e| CryptoError::MalformedMessage(e.to_string()))?;
        let pre_key_message: PreKeyMessage = match message {
            OlmMessage::PreKey(m) => m,
            OlmMessage::Normal(_) => return Err(CryptoError::SessionCreation(
                "expected a pre-key message to establish an inbound session, got a normal message"
                    .to_string(),
            )),
        };
        let result = self
            .inner
            .create_inbound_session(
                SessionConfig::version_1(),
                their_identity_key,
                &pre_key_message,
            )
            .map_err(|e| CryptoError::SessionCreation(e.to_string()))?;
        Ok((
            MySession {
                inner: result.session,
            },
            result.plaintext,
        ))
    }
}

pub struct MySession {
    inner: Session,
}

impl MySession {
    /// Encrypts `plaintext`, returning the `(olm_type, ciphertext)` pair
    /// ready to place in a `pm_proto::Envelope`.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<(u8, Vec<u8>)> {
        let message = self
            .inner
            .encrypt(plaintext)
            .map_err(|e| CryptoError::Encryption(e.to_string()))?;
        let (message_type, ciphertext) = message.to_parts();
        Ok((message_type as u8, ciphertext))
    }

    /// Decrypts a message on an already-established session (i.e., not the
    /// first message from a peer — use [`MyAccount::accept_incoming`] for
    /// that).
    pub fn decrypt(&mut self, olm_type: u8, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let message = OlmMessage::from_parts(olm_type as usize, ciphertext)
            .map_err(|e| CryptoError::MalformedMessage(e.to_string()))?;
        self.inner
            .decrypt(&message)
            .map_err(|e| CryptoError::Decryption(e.to_string()))
    }

    /// Serializes this session's ratchet state for storage, so it can be
    /// restored with [`MySession::from_pickle`] after an app restart. Uses
    /// JSON rather than bincode: vodozemac's pickle types require a
    /// self-describing serde format.
    pub fn pickle(&self) -> Vec<u8> {
        serde_json::to_vec(&self.inner.pickle()).expect("SessionPickle serialization cannot fail")
    }

    /// Restores a session previously serialized with [`MySession::pickle`].
    pub fn from_pickle(bytes: &[u8]) -> Result<Self> {
        let pickle = serde_json::from_slice(bytes)
            .map_err(|e| CryptoError::MalformedMessage(e.to_string()))?;
        Ok(Self {
            inner: Session::from_pickle(pickle),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M1 exit criteria: two in-process identities exchange encrypted
    /// messages. Persistence is covered separately in `pm-store`.
    #[test]
    fn two_accounts_exchange_messages_in_both_directions() {
        let alice = MyAccount::new();
        let mut bob = MyAccount::new();

        let bob_otks = bob.generate_one_time_keys(1);
        let bob_otk = *bob_otks.values().next().unwrap();

        let mut alice_session = alice
            .create_outbound_session(bob.curve25519_key(), bob_otk)
            .unwrap();

        let (olm_type, ciphertext) = alice_session.encrypt(b"hello bob").unwrap();

        let (mut bob_session, plaintext) = bob
            .accept_incoming(alice.curve25519_key(), olm_type, &ciphertext)
            .unwrap();
        assert_eq!(plaintext, b"hello bob");

        // Now bob replies on the now-established session, and alice decrypts
        // as a normal (non-prekey) message.
        let (olm_type, ciphertext) = bob_session.encrypt(b"hi alice").unwrap();
        let reply = alice_session.decrypt(olm_type, &ciphertext).unwrap();
        assert_eq!(reply, b"hi alice");

        // And a second message from alice, also now a normal message.
        let (olm_type, ciphertext) = alice_session.encrypt(b"how are you").unwrap();
        let second = bob_session.decrypt(olm_type, &ciphertext).unwrap();
        assert_eq!(second, b"how are you");
    }

    #[test]
    fn session_survives_a_pickle_roundtrip_mid_conversation() {
        let alice = MyAccount::new();
        let mut bob = MyAccount::new();
        let bob_otks = bob.generate_one_time_keys(1);
        let bob_otk = *bob_otks.values().next().unwrap();

        let mut alice_session = alice
            .create_outbound_session(bob.curve25519_key(), bob_otk)
            .unwrap();
        let (olm_type, ciphertext) = alice_session.encrypt(b"first").unwrap();
        let (bob_session, _) = bob
            .accept_incoming(alice.curve25519_key(), olm_type, &ciphertext)
            .unwrap();

        // Simulate an app restart: serialize bob's session, drop it, and
        // restore a fresh instance from just the pickled bytes.
        let pickled = bob_session.pickle();
        drop(bob_session);
        let mut restored_bob_session = MySession::from_pickle(&pickled).unwrap();

        let (olm_type, ciphertext) = alice_session.encrypt(b"still there?").unwrap();
        let plaintext = restored_bob_session.decrypt(olm_type, &ciphertext).unwrap();
        assert_eq!(plaintext, b"still there?");
    }

    #[test]
    fn accept_incoming_rejects_a_normal_message() {
        let alice = MyAccount::new();
        let mut bob = MyAccount::new();
        let bob_otks = bob.generate_one_time_keys(1);
        let bob_otk = *bob_otks.values().next().unwrap();

        let mut alice_session = alice
            .create_outbound_session(bob.curve25519_key(), bob_otk)
            .unwrap();
        // First message establishes bob's inbound session normally...
        let (olm_type, ciphertext) = alice_session.encrypt(b"first").unwrap();
        let (_bob_session, _) = bob
            .accept_incoming(alice.curve25519_key(), olm_type, &ciphertext)
            .unwrap();

        // ...a second message from alice is a Normal message, which must not
        // be usable to (re-)establish an inbound session.
        let (olm_type, ciphertext) = alice_session.encrypt(b"second").unwrap();
        let mut fresh_bob = MyAccount::new();
        assert!(fresh_bob
            .accept_incoming(alice.curve25519_key(), olm_type, &ciphertext)
            .is_err());
    }
}
