//! Generic per-chain key derivation: `derive(key, label, n)` deterministically
//! produces the nth value in a labeled HKDF chain rooted at `key`.
//!
//! This is the shared primitive underneath any future per-contact-pair
//! derivation (message keys, pointer-record keys, and so on) — see
//! `docs/PRD.md` and `docs/THREAT-MODEL.md` for the open question of whether
//! the original slot-hash/tag-padding scheme built on top of a primitive like
//! this is still needed now that mailboxes are single-tenant.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::error::{ProtoError, Result};

/// Derives `out_len` bytes at position `n` in the HKDF chain identified by
/// `label`, rooted at `key`. Deterministic: the same inputs always produce
/// the same output.
pub fn derive(key: &[u8], label: &str, n: u64, out_len: usize) -> Result<Vec<u8>> {
    let hk = Hkdf::<Sha256>::new(None, key);
    let mut info = Vec::with_capacity(label.len() + 8);
    info.extend_from_slice(label.as_bytes());
    info.extend_from_slice(&n.to_be_bytes());

    let mut okm = vec![0u8; out_len];
    hk.expand(&info, &mut okm)
        .map_err(|_| ProtoError::DerivationLengthInvalid(out_len))?;
    Ok(okm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Pinned test vector. Computed once from the implementation above and
    /// hardcoded here so any future change that alters derivation output is
    /// caught as a regression. Re-derive deliberately (and update this
    /// constant, with a comment explaining why) if the derivation scheme
    /// itself is intentionally changed.
    #[test]
    fn matches_pinned_test_vector() {
        let key = [0x42u8; 32];
        let out = derive(&key, "test-vector", 7, 32).unwrap();
        assert_eq!(
            hex::encode(&out),
            "542cb5995c2bd0636fb6fe77523a7c8274d9426b07e1e3db21b493ba71cf9fa7"
        );
    }

    proptest! {
        #[test]
        fn deterministic_for_same_inputs(
            key in proptest::collection::vec(any::<u8>(), 16..64),
            n in any::<u64>(),
        ) {
            let a = derive(&key, "chain-a", n, 32).unwrap();
            let b = derive(&key, "chain-a", n, 32).unwrap();
            prop_assert_eq!(a, b);
        }

        #[test]
        fn different_n_produces_different_output(
            key in proptest::collection::vec(any::<u8>(), 16..64),
            n in 0u64..1_000_000,
        ) {
            let a = derive(&key, "chain-a", n, 32).unwrap();
            let b = derive(&key, "chain-a", n + 1, 32).unwrap();
            prop_assert_ne!(a, b);
        }

        #[test]
        fn different_labels_produce_different_output(
            key in proptest::collection::vec(any::<u8>(), 16..64),
            n in any::<u64>(),
        ) {
            let a = derive(&key, "chain-a", n, 32).unwrap();
            let b = derive(&key, "chain-b", n, 32).unwrap();
            prop_assert_ne!(a, b);
        }

        #[test]
        fn output_length_matches_request(
            key in proptest::collection::vec(any::<u8>(), 16..64),
            n in any::<u64>(),
            len in 1usize..255,
        ) {
            let out = derive(&key, "chain-a", n, len).unwrap();
            prop_assert_eq!(out.len(), len);
        }
    }
}
