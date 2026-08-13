//! The plaintext wrapper that actually goes into the Olm-encrypted slot,
//! on both delivery paths (Server-mailbox and Local direct). Distinguishes
//! a real chat message from a delivery receipt — `docs/PRD.md` Flow 3's
//! "delivered" status needs *something* to flow back from recipient to
//! sender, and this is that something. Deliberately not a wire-format
//! change to `pm_proto::Envelope` itself — just a convention for how its
//! plaintext is interpreted, entirely internal to `pm-core`. FFI callers
//! only ever see raw chat bytes in and a `status` field on stored
//! messages, never this wrapper.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PlaintextPayload {
    Chat(Vec<u8>),
    Receipt { msg_id: [u8; 16] },
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
    fn garbage_bytes_deserialize_to_none_not_panic() {
        // An enum discriminant (99) that matches neither Chat (0) nor
        // Receipt (1) — must be rejected cleanly, not panic.
        let bad_discriminant = 99u32.to_le_bytes();
        assert!(PlaintextPayload::deserialize(&bad_discriminant).is_none());
        assert!(PlaintextPayload::deserialize(&[]).is_none());
    }
}
