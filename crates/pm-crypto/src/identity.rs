//! Deterministic key derivation from the recovery-phrase seed. Every key
//! here is recoverable on a new device from the seed phrase alone — unlike
//! vodozemac's own session keys (`session.rs`), which are randomly
//! generated per `ARCHIT_1.MD` §4.6 ("sessions re-key rather than restoring
//! old ratchet state").

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::seed::Seed;

const IDENTITY_LABEL: &[u8] = b"pm-identity-v1";
const BACKUP_LABEL: &[u8] = b"pm-backup-v1";
const MAILBOX_LABEL: &[u8] = b"pm-mailbox-v1";
const BACKUP_LOCATION_LABEL: &[u8] = b"pm-backuploc-v1";

fn hkdf_expand_32(ikm: &[u8], label: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 32];
    hk.expand(label, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}

/// The set of keys derived from a user's seed. `signing_key` (IK) is the
/// long-term identity used to sign pairing QR payloads and mailbox-pointer
/// updates (see `docs/PRD.md` §8, "Signed mailbox-pointer updates"). The
/// other three are raw key material for backup encryption, mailbox
/// authentication, and the backup-location pointer, respectively.
pub struct Identity {
    pub signing_key: SigningKey,
    pub backup_key: [u8; 32],
    pub mailbox_key: [u8; 32],
    pub backup_location_key: [u8; 32],
}

impl Identity {
    pub fn derive(seed: &Seed) -> Self {
        let identity_bytes = hkdf_expand_32(&seed.0, IDENTITY_LABEL);
        Self {
            signing_key: SigningKey::from_bytes(&identity_bytes),
            backup_key: hkdf_expand_32(&seed.0, BACKUP_LABEL),
            mailbox_key: hkdf_expand_32(&seed.0, MAILBOX_LABEL),
            backup_location_key: hkdf_expand_32(&seed.0, BACKUP_LOCATION_LABEL),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, Verifier};

    fn fixed_seed() -> Seed {
        Seed([0x11u8; 32])
    }

    #[test]
    fn derivation_is_deterministic() {
        let a = Identity::derive(&fixed_seed());
        let b = Identity::derive(&fixed_seed());
        assert_eq!(a.signing_key.to_bytes(), b.signing_key.to_bytes());
        assert_eq!(a.backup_key, b.backup_key);
        assert_eq!(a.mailbox_key, b.mailbox_key);
        assert_eq!(a.backup_location_key, b.backup_location_key);
    }

    #[test]
    fn the_four_derived_keys_are_pairwise_distinct() {
        let id = Identity::derive(&fixed_seed());
        let ik_bytes = id.signing_key.to_bytes();
        assert_ne!(ik_bytes, id.backup_key);
        assert_ne!(ik_bytes, id.mailbox_key);
        assert_ne!(ik_bytes, id.backup_location_key);
        assert_ne!(id.backup_key, id.mailbox_key);
        assert_ne!(id.backup_key, id.backup_location_key);
        assert_ne!(id.mailbox_key, id.backup_location_key);
    }

    #[test]
    fn different_seeds_produce_different_identities() {
        let a = Identity::derive(&fixed_seed());
        let b = Identity::derive(&Seed([0x22u8; 32]));
        assert_ne!(a.signing_key.to_bytes(), b.signing_key.to_bytes());
    }

    #[test]
    fn signing_key_matches_pinned_test_vector() {
        let id = Identity::derive(&fixed_seed());
        assert_eq!(
            hex::encode(id.signing_key.verifying_key().to_bytes()),
            "7733840ca113e81a646b8d96b6f2abba88f88a6fbcb4133add4fda7186e118dd"
        );
    }

    #[test]
    fn signing_key_can_sign_and_verify() {
        let id = Identity::derive(&fixed_seed());
        let message = b"pairing payload";
        let signature = id.signing_key.sign(message);
        assert!(id
            .signing_key
            .verifying_key()
            .verify(message, &signature)
            .is_ok());
    }
}
