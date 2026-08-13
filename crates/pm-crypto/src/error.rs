use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid recovery phrase: {0}")]
    InvalidMnemonic(String),

    #[error("vodozemac session creation failed: {0}")]
    SessionCreation(String),

    #[error("vodozemac encryption failed: {0}")]
    Encryption(String),

    #[error("vodozemac decryption failed: {0}")]
    Decryption(String),

    #[error("malformed Olm message: {0}")]
    MalformedMessage(String),
}

pub type Result<T> = core::result::Result<T, CryptoError>;
