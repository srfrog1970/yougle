//! The message envelope: the structure encrypted end-to-end between two
//! identities, before the outer padding-and-AEAD layer (`pm-crypto`) hides
//! its framing from any mailbox that stores it.

use serde::{Deserialize, Serialize};

use crate::error::{ProtoError, Result};
use crate::padding;

/// Current wire-format version. Bump when the envelope layout changes in a
/// way that isn't backward compatible.
pub const ENVELOPE_VERSION: u8 = 1;

/// Reserved for MVP: attachment references. Always empty in the MVP client,
/// per `docs/PRD.md` (attachments are explicitly out of scope), but the
/// field exists now so the wire format doesn't need to change to add them
/// later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub id: [u8; 16],
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u8,
    /// Placeholder discriminant for the underlying Olm message type
    /// (prekey vs. normal), pending `pm-crypto`.
    pub olm_type: u8,
    pub ciphertext: Vec<u8>,
    /// Position in the sender's per-direction chain.
    pub n: u64,
    pub lamport: u64,
    /// Unix milliseconds.
    pub sent_at: u64,
    pub msg_id: [u8; 16],
    /// Always empty in the MVP. See `AttachmentRef`.
    pub attachments: Vec<AttachmentRef>,
}

impl Envelope {
    pub fn new(
        olm_type: u8,
        ciphertext: Vec<u8>,
        n: u64,
        lamport: u64,
        sent_at: u64,
        msg_id: [u8; 16],
    ) -> Self {
        Self {
            version: ENVELOPE_VERSION,
            olm_type,
            ciphertext,
            n,
            lamport,
            sent_at,
            msg_id,
            attachments: Vec::new(),
        }
    }

    /// Serializes this envelope and pads the result to a fixed bucket size,
    /// so its length doesn't leak message size once encrypted.
    pub fn to_padded_bytes(&self) -> Result<Vec<u8>> {
        let bytes =
            bincode::serialize(self).map_err(|e| ProtoError::Serialization(e.to_string()))?;
        padding::pad(&bytes)
    }

    /// Reverses [`Envelope::to_padded_bytes`].
    pub fn from_padded_bytes(padded: &[u8]) -> Result<Self> {
        let bytes = padding::unpad(padded)?;
        bincode::deserialize(&bytes).map_err(|e| ProtoError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_envelope(ciphertext_len: usize) -> Envelope {
        Envelope::new(
            0,
            vec![0xAB; ciphertext_len],
            42,
            100,
            1_754_000_000_000,
            [7u8; 16],
        )
    }

    #[test]
    fn new_envelope_has_no_attachments() {
        let env = sample_envelope(10);
        assert!(env.attachments.is_empty());
    }

    #[test]
    fn new_envelope_carries_current_version() {
        let env = sample_envelope(10);
        assert_eq!(env.version, ENVELOPE_VERSION);
    }

    #[test]
    fn roundtrips_through_padded_bytes() {
        let env = sample_envelope(50);
        let padded = env.to_padded_bytes().unwrap();
        let recovered = Envelope::from_padded_bytes(&padded).unwrap();
        assert_eq!(env, recovered);
    }

    #[test]
    fn ciphertext_too_large_for_any_bucket_errors() {
        let env = sample_envelope(padding::BUCKETS[2] + 1);
        assert!(env.to_padded_bytes().is_err());
    }

    proptest! {
        #[test]
        fn roundtrips_for_varied_ciphertext_sizes(len in 0usize..4000) {
            let env = sample_envelope(len);
            if let Ok(padded) = env.to_padded_bytes() {
                let recovered = Envelope::from_padded_bytes(&padded).unwrap();
                prop_assert_eq!(env, recovered);
            }
        }
    }
}
