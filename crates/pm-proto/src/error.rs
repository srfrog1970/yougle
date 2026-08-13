use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("data too large to pad: {len} bytes exceeds the largest bucket ({max} bytes)")]
    TooLargeToPad { len: usize, max: usize },

    #[error("padded buffer too short to contain a length prefix")]
    PaddedBufferTooShort,

    #[error("padded buffer's declared length ({declared}) exceeds its actual size ({actual})")]
    PaddedLengthInvalid { declared: usize, actual: usize },

    #[error("requested derivation output length ({0}) is invalid for HKDF-SHA256")]
    DerivationLengthInvalid(usize),

    #[error("serialization failed: {0}")]
    Serialization(String),
}

pub type Result<T> = core::result::Result<T, ProtoError>;
