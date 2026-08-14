//! Encrypts/decrypts an opaque backup blob under a user's `backup_key`
//! (`ARCHIT_1.MD` §4.1/§4.6). What goes *inside* the blob (contacts,
//! session state, message history) is `pm-core`'s concern — this module
//! only handles the encryption envelope, matching `docs/PRD.md`'s locked
//! choice of XChaCha20-Poly1305 for backups.

use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{AeadCore, Key, KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::{CryptoError, Result};

const NONCE_LEN: usize = 24;

/// Padding floor, in bytes — matches `pm-proto`'s own largest fixed
/// envelope bucket, so a tiny or empty backup doesn't stand out as
/// obviously small either. See `pad`'s own doc comment for why backups
/// need a different (unbounded, power-of-two) scheme than `pm-proto`'s
/// fixed 3-bucket one, which is sized for a single envelope, not a full
/// message history.
const MIN_PADDED_LEN: usize = 4096;

/// 8 bytes, not `pm-proto`'s 4 — a backup has no fixed size cap the way a
/// single envelope does (capped at ~4 KB there), so a `u32` length prefix
/// could in principle silently truncate (and corrupt) a large enough
/// backup instead of erroring. `u64` removes the concern entirely, at a
/// cost of 4 negligible extra bytes against the 4096-byte floor above.
const LEN_PREFIX_SIZE: usize = 8;

/// Pads `data` up to the next power of two (floored at [`MIN_PADDED_LEN`])
/// that fits an 8-byte big-endian length prefix plus `data` itself.
///
/// Deliberately not `pm-proto`'s own envelope padding (`pm_proto::padding`,
/// fixed `[256, 1024, 4096]` buckets): that scheme is sized for one
/// message, and errors outright on anything larger than ~4 KB — a backup
/// (full message history, contacts, session state) is fundamentally
/// unbounded, so bucket boundaries need to scale with the input instead of
/// being fixed. Power-of-two bucketing keeps that property at any size:
/// the padded length only ever reveals "somewhere in this power-of-two
/// range," never the exact byte count, at up to ~2x space overhead in the
/// worst case (right after crossing a boundary) — the same overhead-for-
/// privacy tradeoff `pm-proto`'s own envelope padding already accepts (its
/// smallest bucket is 256 B even for a 1-byte message).
fn pad(data: &[u8]) -> Vec<u8> {
    let needed = data.len() + LEN_PREFIX_SIZE;
    let bucket = needed.next_power_of_two().max(MIN_PADDED_LEN);

    let mut out = Vec::with_capacity(bucket);
    out.extend_from_slice(&(data.len() as u64).to_be_bytes());
    out.extend_from_slice(data);
    out.resize(bucket, 0);
    out
}

/// Reverses [`pad`].
fn unpad(padded: &[u8]) -> Result<Vec<u8>> {
    if padded.len() < LEN_PREFIX_SIZE {
        return Err(CryptoError::Decryption(
            "padded backup buffer shorter than its length prefix".to_string(),
        ));
    }
    let declared = u64::from_be_bytes(padded[0..LEN_PREFIX_SIZE].try_into().unwrap()) as usize;
    let end = LEN_PREFIX_SIZE + declared;
    if end > padded.len() {
        return Err(CryptoError::Decryption(
            "padded backup length prefix exceeds buffer".to_string(),
        ));
    }
    Ok(padded[LEN_PREFIX_SIZE..end].to_vec())
}

/// Encrypts `plaintext` under `backup_key`, returning `nonce ||
/// ciphertext`. `plaintext` is padded first (see [`pad`]) so ciphertext
/// length reveals only a coarse size class, not the exact byte count —
/// same reason `pm-proto`'s envelope padding exists, applied here since a
/// backup file's size would otherwise be a rough proxy for how much
/// message history it contains, to anyone who sees the file (a Server
/// mailbox storing it, or wherever a manually-exported one ends up).
pub fn encrypt_backup(backup_key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let padded = pad(plaintext);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(backup_key));
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, padded.as_slice())
        .expect("XChaCha20-Poly1305 encryption of a well-formed plaintext cannot fail");

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

/// Reverses [`encrypt_backup`]. Fails if `blob` is too short to contain a
/// nonce, if authentication fails (wrong key, or corrupted/tampered data),
/// or if the decrypted plaintext isn't validly padded.
pub fn decrypt_backup(backup_key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        return Err(CryptoError::Decryption(
            "backup blob shorter than a nonce".to_string(),
        ));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(backup_key));
    let nonce = XNonce::from_slice(nonce_bytes);
    let padded = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::Decryption("backup decryption failed".to_string()))?;
    unpad(&padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let key = [7u8; 32];
        let plaintext = b"contacts and session state, in some serialized form";
        let blob = encrypt_backup(&key, plaintext);
        assert_eq!(decrypt_backup(&key, &blob).unwrap(), plaintext);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let blob = encrypt_backup(&[7u8; 32], b"secret");
        assert!(decrypt_backup(&[9u8; 32], &blob).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails_to_decrypt() {
        let mut blob = encrypt_backup(&[7u8; 32], b"secret");
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;
        assert!(decrypt_backup(&[7u8; 32], &blob).is_err());
    }

    #[test]
    fn two_encryptions_of_the_same_plaintext_differ() {
        let key = [7u8; 32];
        let a = encrypt_backup(&key, b"secret");
        let b = encrypt_backup(&key, b"secret");
        assert_ne!(a, b, "a fresh random nonce must be used each time");
    }

    #[test]
    fn an_empty_backup_still_pads_up_to_the_floor() {
        let key = [7u8; 32];
        let blob = encrypt_backup(&key, b"");
        assert_eq!(
            blob.len(),
            NONCE_LEN + MIN_PADDED_LEN + 16 /* Poly1305 tag */
        );
    }

    #[test]
    fn differently_sized_plaintexts_in_the_same_bucket_produce_identical_lengths() {
        let key = [7u8; 32];
        let small = encrypt_backup(&key, &[0u8; 10]);
        let bigger = encrypt_backup(&key, &[0u8; 1000]);
        assert_eq!(
            small.len(),
            bigger.len(),
            "both fit MIN_PADDED_LEN's bucket, so their sizes must be indistinguishable"
        );
    }

    #[test]
    fn plaintexts_straddling_a_bucket_boundary_produce_different_lengths() {
        let key = [7u8; 32];
        // MIN_PADDED_LEN (4096) fits up to 4096 - 8 = 4088 bytes of
        // plaintext before the next power-of-two bucket (8192) is needed.
        let just_fits = encrypt_backup(&key, &vec![0u8; 4088]);
        let just_over = encrypt_backup(&key, &vec![0u8; 4089]);
        assert!(
            just_over.len() > just_fits.len(),
            "padding must still scale across a bucket boundary, not hide magnitude entirely"
        );
    }

    #[test]
    fn a_large_backup_pads_to_a_power_of_two_above_the_floor() {
        let key = [7u8; 32];
        let plaintext = vec![0u8; 100_000];
        let blob = encrypt_backup(&key, &plaintext);
        let padded_len = blob.len() - NONCE_LEN - 16 /* Poly1305 tag */;
        assert!(padded_len.is_power_of_two());
        assert!(padded_len >= plaintext.len() + LEN_PREFIX_SIZE);
    }
}
