//! Wire protocol for direct client-to-client delivery (Local-to-local, per
//! `docs/PRD.md` Flow 3's second case), ALPN [`DIRECT_ALPN`]. One envelope
//! per connection: the sender opens a bidirectional stream, writes the
//! padded envelope bytes, finishes the send side, then reads a
//! [`DirectAck`] to end; the receiver mirrors that — deliberately simpler
//! than [`crate::node_protocol`]'s request/response shape, since there's
//! no mailbox-ownership concept here: whoever's listening on this ALPN
//! accepts, tries to decrypt against its known contacts, and reports back
//! whether that worked.

use serde::{Deserialize, Serialize};

pub const DIRECT_ALPN: &[u8] = b"pm/direct/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DirectAck {
    Ok,
    Error(String),
}
