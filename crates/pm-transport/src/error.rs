use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("endpoint setup failed: {0}")]
    Endpoint(String),

    #[error("connection failed: {0}")]
    Connect(String),

    #[error("stream I/O failed: {0}")]
    Stream(String),

    #[error("failed to encode/decode a node message: {0}")]
    Codec(String),

    #[error("peer rejected the delivery: {0}")]
    Rejected(String),
}

pub type Result<T> = core::result::Result<T, TransportError>;
