//! `pm-core`: the client state machine and sync engine tying `pm-proto`,
//! `pm-crypto`, `pm-store`, and `pm-transport` together into one API. See
//! `client.rs` for the scope note on what's covered in M3 versus deferred.

pub mod backup;
pub mod client;
pub mod error;

pub use backup::{BackupBundle, BackupContact};
pub use client::Client;
pub use error::{CoreError, Result};
