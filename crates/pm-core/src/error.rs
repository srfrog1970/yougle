use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Store(#[from] pm_store::StoreError),

    #[error(transparent)]
    Crypto(#[from] pm_crypto::CryptoError),

    #[error(transparent)]
    Proto(#[from] pm_proto::ProtoError),

    #[error(transparent)]
    Transport(#[from] pm_transport::TransportError),

    #[error("unknown contact id {0}")]
    UnknownContact(i64),

    #[error("sending to this contact requires a known Server mailbox address; direct Local delivery isn't wired into pm-core yet (deferred to M4/M5)")]
    NoServerForContact,

    #[error("this client has no Server mailbox configured, so there's nothing to sync")]
    NoOwnServer,

    #[error("could not attribute an incoming message to any known contact")]
    UnattributedMessage,

    #[error("no backup found at that Server address")]
    NoBackupFound,

    #[error("backup bundle is corrupt: {0}")]
    CorruptBackup(String),

    #[error("node returned an error: {0}")]
    NodeError(String),
}

pub type Result<T> = core::result::Result<T, CoreError>;
