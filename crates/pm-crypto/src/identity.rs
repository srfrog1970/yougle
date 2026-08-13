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
const TRANSPORT_LABEL: &[u8] = b"pm-transport-v1";
const SERVER_TRANSPORT_LABEL: &[u8] = b"pm-transport-server-v1";

fn hkdf_expand_32(ikm: &[u8], label: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = [0u8; 32];
    hk.expand(label, &mut okm)
        .expect("32 is a valid HKDF-SHA256 output length");
    okm
}

/// The set of keys derived from a user's seed. `signing_key` (IK) is the
/// long-term identity used to sign pairing QR payloads and mailbox-pointer
/// updates (see `docs/PRD.md` §8, "Signed mailbox-pointer updates").
///
/// `transport_key` and `server_transport_key` are both iroh endpoint
/// *secrets* (each seeds an `iroh::SecretKey`, never shared as-is), kept as
/// two *separate* derived keys rather than one shared value:
/// `transport_key` seeds this device's own Local accept loop — its
/// corresponding *public* id is what gets shared with contacts at pairing
/// time, so they can dial this device directly, see M6's
/// `Client::pairing_payload` — while `server_transport_key` is only ever
/// used if this same person also runs their own self-hosted `pm-node`. Those
/// are two
/// different physical processes that can genuinely be running at the same
/// time (phone app open *and* home server up) — deriving the same iroh
/// identity for both would mean two different machines simultaneously
/// claiming the same network identity, which iroh has no defined behavior
/// for. Neither is reused from `signing_key` either, so the transport/TLS
/// layer never shares key material with application-level signing. The
/// remaining two are raw key material for backup encryption and mailbox
/// authentication.
pub struct Identity {
    pub signing_key: SigningKey,
    pub backup_key: [u8; 32],
    pub mailbox_key: [u8; 32],
    pub backup_location_key: [u8; 32],
    pub transport_key: [u8; 32],
    pub server_transport_key: [u8; 32],
}

impl Identity {
    pub fn derive(seed: &Seed) -> Self {
        let identity_bytes = hkdf_expand_32(&seed.0, IDENTITY_LABEL);
        Self {
            signing_key: SigningKey::from_bytes(&identity_bytes),
            backup_key: hkdf_expand_32(&seed.0, BACKUP_LABEL),
            mailbox_key: hkdf_expand_32(&seed.0, MAILBOX_LABEL),
            backup_location_key: hkdf_expand_32(&seed.0, BACKUP_LOCATION_LABEL),
            transport_key: hkdf_expand_32(&seed.0, TRANSPORT_LABEL),
            server_transport_key: hkdf_expand_32(&seed.0, SERVER_TRANSPORT_LABEL),
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
        assert_eq!(a.transport_key, b.transport_key);
        assert_eq!(a.server_transport_key, b.server_transport_key);
    }

    #[test]
    fn all_derived_keys_are_pairwise_distinct() {
        let id = Identity::derive(&fixed_seed());
        let keys = [
            id.signing_key.to_bytes(),
            id.backup_key,
            id.mailbox_key,
            id.backup_location_key,
            id.transport_key,
            id.server_transport_key,
        ];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "keys at index {i} and {j} collide");
            }
        }
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
