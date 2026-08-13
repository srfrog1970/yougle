use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("message id must be exactly 16 bytes, got {0}")]
    InvalidMsgIdLength(usize),
}

pub type Result<T> = core::result::Result<T, StoreError>;
