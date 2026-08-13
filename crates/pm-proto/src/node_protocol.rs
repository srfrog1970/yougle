//! The wire protocol spoken between a client (`pm-transport`, eventually
//! `pm-core`) and a Server mailbox (`pm-node`) over iroh QUIC, ALPN
//! [`NODE_ALPN`]. One request/response pair per QUIC connection: the client
//! opens a bidirectional stream, writes the serialized request, finishes the
//! send side, then reads the response to end; the server mirrors that.
//!
//! **Deliberately simplified from `ARCHIT_1.MD`'s original node API.** That
//! design (`REDEEM_TICKET`, slots padded to a fixed 4096 count) existed to
//! hide contact-graph information from a shared, third-party-operated
//! community mailbox. `docs/PRD.md` v2.0 removed community mailboxes
//! entirely — every mailbox is single-tenant and owned by the person running
//! it — so that padding has no one left to hide from. What's kept here is
//! the part that's still needed regardless of that open question: proof of
//! authorization to write, via a pre-registered hash whose preimage only a
//! paired sender can produce. Whether anything closer to the original
//! anti-enumeration scheme is still warranted is tracked as an open item in
//! `docs/PRD.md` and `docs/THREAT-MODEL.md`, not resolved here.

use serde::{Deserialize, Serialize};

pub const NODE_ALPN: &[u8] = b"pm/node/1";

/// Generous upper bound on a single request/response message, so a
/// misbehaving peer can't force unbounded buffering.
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeRequest {
    /// Stages one acceptable proof-of-write hash. `slot_hash` is
    /// `SHA256(auth)` for some `auth` the registering owner expects a sender
    /// to later present. Authenticated by mailbox ownership (`mailbox_key`)
    /// since only the owner should control what can be written to their own
    /// inbox.
    RegisterSlot {
        mailbox_key: [u8; 32],
        slot_hash: [u8; 32],
    },
    /// Deposits `blob` if `SHA256(auth)` matches a currently-registered
    /// slot. The matched slot is consumed (one-time use, preventing
    /// replay). No mailbox-owner identity is presented or required — this
    /// is the call a sender, who isn't the mailbox owner, makes.
    Write { auth: [u8; 32], blob: Vec<u8> },
    /// Retrieves everything currently stored, delivered or not.
    /// Authenticated by mailbox ownership.
    Fetch { mailbox_key: [u8; 32] },
    /// Marks the given stored blobs as delivered. Per `docs/PRD.md` §5/§8,
    /// this does *not* delete them — Server mailboxes retain delivered
    /// messages to support planned functionality built on that data (see
    /// the Open Items: a retention/quota policy is still needed).
    /// Authenticated by mailbox ownership.
    Ack {
        mailbox_key: [u8; 32],
        ids: Vec<u64>,
    },
    /// Stores (replacing any previous value) an opaque encrypted backup
    /// blob for this mailbox's owner — per `ARCHIT_1.MD` §4.6 and
    /// `docs/PRD.md` §3/§5, the contacts-and-history bundle a new device
    /// restores from. Authenticated by mailbox ownership.
    PutBackup {
        mailbox_key: [u8; 32],
        blob: Vec<u8>,
    },
    /// Retrieves the current backup blob, if any. Authenticated by mailbox
    /// ownership — unlike the original architecture's DHT-published
    /// pointer-then-fetch design, restoring on a new device in this build
    /// means re-supplying the Server's address directly (see
    /// `pm-core`'s docs for why: DHT publishing is an open item, not
    /// resolved here), so the requester already needs to know the mailbox
    /// key to ask at all.
    GetBackup { mailbox_key: [u8; 32] },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredBlob {
    pub id: u64,
    pub blob: Vec<u8>,
    pub delivered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeResponse {
    Ok,
    Blobs(Vec<StoredBlob>),
    /// `None` if no backup has ever been stored for this mailbox.
    Backup(Option<Vec<u8>>),
    Error(String),
}
