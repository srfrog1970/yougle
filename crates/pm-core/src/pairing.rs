//! Mutual pairing: each device shares one [`PairingPayload`] (its public
//! keys, a one-time key, and a fresh random nonce) and, from the two
//! payloads, both independently derive the same `pair_secret` — without
//! either side needing to know who scanned/pasted first.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// What one device shares with a pairing partner — via QR or a pasted
/// text code (the RN layer's job, not this crate's). `server_addr` is the
/// opaque, `pm-core`-interpreted bytes also used elsewhere (see
/// `pm_store::ContactRecord::server_addr`), included so the other side can
/// message this device immediately without a separate exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    pub identity_key: [u8; 32],
    pub curve25519_key: [u8; 32],
    pub one_time_key: [u8; 32],
    pub nonce: [u8; 32],
    pub server_addr: Option<Vec<u8>>,
}

/// Derives the shared `pair_secret` from both sides' nonces. Sorting the
/// two nonces before combining makes this order-independent — either side
/// can compute it first, and whoever scans/pastes second gets the same
/// result, without a round-trip to agree on an order.
///
/// Built on `pm_proto::derive`, the same HKDF chain-derivation primitive
/// used for message-key and pointer-record chains — no new crypto
/// primitive, just a new composition of an existing, already-tested one.
pub fn compute_pair_secret(nonce_a: [u8; 32], nonce_b: [u8; 32]) -> Result<[u8; 32]> {
    let (first, second) = if nonce_a <= nonce_b {
        (nonce_a, nonce_b)
    } else {
        (nonce_b, nonce_a)
    };
    let mut combined = Vec::with_capacity(64);
    combined.extend_from_slice(&first);
    combined.extend_from_slice(&second);

    let out = pm_proto::derive(&combined, "pairing", 0, 32)?;
    Ok(out
        .try_into()
        .expect("derive() with out_len=32 always returns exactly 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_independent() {
        let a = [0x11u8; 32];
        let b = [0x22u8; 32];
        assert_eq!(
            compute_pair_secret(a, b).unwrap(),
            compute_pair_secret(b, a).unwrap(),
            "either side computing first must land on the same secret"
        );
    }

    #[test]
    fn different_nonce_pairs_produce_different_secrets() {
        let a = [0x11u8; 32];
        let b = [0x22u8; 32];
        let c = [0x33u8; 32];
        assert_ne!(
            compute_pair_secret(a, b).unwrap(),
            compute_pair_secret(a, c).unwrap()
        );
    }

    #[test]
    fn deterministic() {
        let a = [0x11u8; 32];
        let b = [0x22u8; 32];
        assert_eq!(
            compute_pair_secret(a, b).unwrap(),
            compute_pair_secret(a, b).unwrap()
        );
    }
}
