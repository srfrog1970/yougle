//! BIP39 recovery-phrase handling. Per `docs/PRD.md`, identity is always
//! recoverable from the 24-word phrase alone — everything in `identity.rs`
//! is derived deterministically from the 32 bytes of entropy those 24 words
//! encode (not the 64-byte BIP32-style `to_seed` value, which is a different
//! BIP39 output meant for HD wallet derivation and isn't what we want here).

use bip39::Mnemonic;

use crate::error::{CryptoError, Result};

pub const SEED_LEN: usize = 32;
const WORD_COUNT: usize = 24;

#[derive(Clone)]
pub struct Seed(pub [u8; SEED_LEN]);

impl Seed {
    /// Generates a fresh 24-word recovery phrase and the seed it encodes.
    pub fn generate() -> (Self, Mnemonic) {
        let mnemonic = Mnemonic::generate(WORD_COUNT)
            .expect("24 is a valid BIP39 word count, so generation cannot fail");
        let seed =
            Self::from_mnemonic(&mnemonic).expect("a freshly generated mnemonic is always valid");
        (seed, mnemonic)
    }

    /// Recovers the seed from a previously recorded recovery phrase.
    pub fn from_phrase(phrase: &str) -> Result<Self> {
        let mnemonic =
            Mnemonic::parse(phrase).map_err(|e| CryptoError::InvalidMnemonic(e.to_string()))?;
        Self::from_mnemonic(&mnemonic)
    }

    fn from_mnemonic(mnemonic: &Mnemonic) -> Result<Self> {
        let entropy = mnemonic.to_entropy();
        let bytes: [u8; SEED_LEN] = entropy.as_slice().try_into().map_err(|_| {
            CryptoError::InvalidMnemonic(format!(
                "expected a {SEED_LEN}-byte seed from a {WORD_COUNT}-word phrase, got {} bytes",
                entropy.len()
            ))
        })?;
        Ok(Seed(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_phrase_has_24_words() {
        let (_, mnemonic) = Seed::generate();
        assert_eq!(mnemonic.word_count(), WORD_COUNT);
    }

    #[test]
    fn recovering_from_the_generated_phrase_yields_the_same_seed() {
        let (seed, mnemonic) = Seed::generate();
        let recovered = Seed::from_phrase(&mnemonic.to_string()).unwrap();
        assert_eq!(seed.0, recovered.0);
    }

    #[test]
    fn garbage_phrase_is_rejected() {
        assert!(Seed::from_phrase("not a valid recovery phrase at all").is_err());
    }
}
