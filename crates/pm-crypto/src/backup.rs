//! Encrypts/decrypts an opaque backup blob under a user's `backup_key`
//! (`ARCHIT_1.MD` §4.1/§4.6). What goes *inside* the blob (contacts,
//! session state, message history) is `pm-core`'s concern — this module
//! only handles the encryption envelope, matching `docs/PRD.md`'s locked
//! choice of XChaCha20-Poly1305 for backups.

use chacha20poly1305::aead::{Aead, OsRng};
use chacha20poly1305::{AeadCore, Key, KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::{CryptoError, Result};

const NONCE_LEN: usize = 24;

/// Encrypts `plaintext` under `backup_key`, returning `nonce || ciphertext`.
pub fn encrypt_backup(backup_key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(backup_key));
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .expect("XChaCha20-Poly1305 encryption of a well-formed plaintext cannot fail");

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

/// Reverses [`encrypt_backup`]. Fails if `blob` is too short to contain a
/// nonce, or if authentication fails (wrong key, or corrupted/tampered
/// data).
pub fn decrypt_backup(backup_key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        return Err(CryptoError::Decryption(
            "backup blob shorter than a nonce".to_string(),
        ));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(backup_key));
    let nonce = XNonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::Decryption("backup decryption failed".to_string()))
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
}
