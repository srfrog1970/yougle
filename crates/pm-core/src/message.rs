//! The plaintext wrapper that actually goes into the Olm-encrypted slot,
//! on both delivery paths (Server-mailbox and Local direct). Distinguishes
//! a real chat message from a delivery receipt — `docs/PRD.md` Flow 3's
//! "delivered" status needs *something* to flow back from recipient to
//! sender, and this is that something — and, since M-pointer-updates,
//! from a signed mailbox-pointer update (`docs/PRD.md` §8). Deliberately
//! not a wire-format change to `pm_proto::Envelope` itself — just a
//! convention for how its plaintext is interpreted, entirely internal to
//! `pm-core`. FFI callers only ever see raw chat bytes in and a `status`
//! field on stored messages, never this wrapper.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PlaintextPayload {
    Chat(Vec<u8>),
    Receipt {
        msg_id: [u8; 16],
    },
    /// A signed announcement that the sender's own mailbox situation
    /// changed — `server_addr: None` means "I'm Local-only now".
    /// `signature` is a 64-byte Ed25519 signature over `(server_addr,
    /// updated_at, seq)` (`Vec<u8>`, not `[u8; 64]` — serde's built-in
    /// array support tops out well below 64), verified independently of
    /// whatever Olm session carries this (see
    /// `client::verify_pointer_update`). `seq` — this sender's own
    /// locally-persisted, monotonically-incrementing broadcast counter —
    /// is the actual replay/rollback protection: a receiver only accepts
    /// one strictly greater than the last it applied from this sender.
    /// `updated_at` is a wall-clock timestamp kept for informational
    /// purposes only (audit/debugging) — deliberately *not* used to gate
    /// acceptance, since wall-clock time isn't guaranteed to move forward
    /// consistently across independent devices (a genuinely later update
    /// can carry an earlier timestamp than one already accepted, e.g.
    /// clock drift), which a monotonic counter is immune to by
    /// construction.
    MailboxPointerUpdate {
        server_addr: Option<Vec<u8>>,
        updated_at: u64,
        seq: u64,
        signature: Vec<u8>,
    },
}

impl PlaintextPayload {
    pub(crate) fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).expect("PlaintextPayload serialization cannot fail")
    }

    /// `None` on malformed input — treated the same as an unattributable
    /// envelope elsewhere in this crate (skipped, not a hard failure),
    /// since a genuine decrypt success against the wrong session is
    /// cryptographically implausible (Olm's AEAD would reject it); this
    /// path only realistically triggers on a version mismatch or bug.
    pub(crate) fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_roundtrips() {
        let payload = PlaintextPayload::Chat(b"hello".to_vec());
        let bytes = payload.serialize();
        match PlaintextPayload::deserialize(&bytes) {
            Some(PlaintextPayload::Chat(text)) => assert_eq!(text, b"hello"),
            other => panic!("expected Chat, got {other:?}"),
        }
    }

    #[test]
    fn receipt_roundtrips() {
        let payload = PlaintextPayload::Receipt { msg_id: [7u8; 16] };
        let bytes = payload.serialize();
        match PlaintextPayload::deserialize(&bytes) {
            Some(PlaintextPayload::Receipt { msg_id }) => assert_eq!(msg_id, [7u8; 16]),
            other => panic!("expected Receipt, got {other:?}"),
        }
    }

    #[test]
    fn mailbox_pointer_update_roundtrips() {
        let payload = PlaintextPayload::MailboxPointerUpdate {
            server_addr: Some(b"fake-endpoint-addr".to_vec()),
            updated_at: 1_754_000_000_000,
            seq: 1,
            signature: vec![9u8; 64],
        };
        let bytes = payload.serialize();
        match PlaintextPayload::deserialize(&bytes) {
            Some(PlaintextPayload::MailboxPointerUpdate {
                server_addr,
                updated_at,
                seq,
                signature,
            }) => {
                assert_eq!(server_addr, Some(b"fake-endpoint-addr".to_vec()));
                assert_eq!(updated_at, 1_754_000_000_000);
                assert_eq!(seq, 1);
                assert_eq!(signature, vec![9u8; 64]);
            }
            other => panic!("expected MailboxPointerUpdate, got {other:?}"),
        }
    }

    #[test]
    fn mailbox_pointer_update_with_no_server_addr_roundtrips() {
        let payload = PlaintextPayload::MailboxPointerUpdate {
            server_addr: None,
            updated_at: 1,
            seq: 1,
            signature: vec![0u8; 64],
        };
        let bytes = payload.serialize();
        match PlaintextPayload::deserialize(&bytes) {
            Some(PlaintextPayload::MailboxPointerUpdate { server_addr, .. }) => {
                assert_eq!(server_addr, None);
            }
            other => panic!("expected MailboxPointerUpdate, got {other:?}"),
        }
    }

    #[test]
    fn garbage_bytes_deserialize_to_none_not_panic() {
        // An enum discriminant (99) that matches neither Chat (0) nor
        // Receipt (1) — must be rejected cleanly, not panic.
        let bad_discriminant = 99u32.to_le_bytes();
        assert!(PlaintextPayload::deserialize(&bad_discriminant).is_none());
        assert!(PlaintextPayload::deserialize(&[]).is_none());
    }
}
