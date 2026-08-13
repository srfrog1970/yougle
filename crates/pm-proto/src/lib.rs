//! `pm-proto`: wire envelopes, chain derivation, and versioning shared
//! verbatim between the Yougle client core (`pm-core`) and the Server
//! mailbox binary (`pm-node`), so framing and derivation logic can never
//! drift between the two.
//!
//! No networking, storage, or crypto-session logic lives here — see
//! `pm-crypto`, `pm-store`, and `pm-transport`.

pub mod derive;
pub mod envelope;
pub mod error;
pub mod padding;

pub use derive::derive;
pub use envelope::{AttachmentRef, Envelope, ENVELOPE_VERSION};
pub use error::{ProtoError, Result};
