//! Dispatches incoming `NodeRequest`s to a [`MailboxStore`], enforcing the
//! one mailbox-owner check every request that touches the owner's inbox
//! requires.

use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use pm_proto::{NodeRequest, NodeResponse, MAX_MESSAGE_SIZE};

use crate::store::{MailboxStore, WriteError};

#[derive(Debug, Clone)]
pub struct MailboxHandler {
    store: Arc<MailboxStore>,
    /// The single tenant this node serves. Requests that claim mailbox
    /// ownership (`Fetch`, `Ack`, `RegisterSlot`) must present this exact
    /// key; `Write` never does, since a sender isn't the owner.
    mailbox_key: [u8; 32],
}

impl MailboxHandler {
    pub fn new(store: Arc<MailboxStore>, mailbox_key: [u8; 32]) -> Self {
        Self { store, mailbox_key }
    }

    fn handle(&self, request: NodeRequest) -> NodeResponse {
        match request {
            NodeRequest::RegisterSlot {
                mailbox_key,
                slot_hash,
            } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                self.store.register_slot(slot_hash);
                NodeResponse::Ok
            }
            NodeRequest::Write { auth, blob } => match self.store.write(auth, blob) {
                Ok(_id) => NodeResponse::Ok,
                Err(WriteError::NoMatchingSlot) => {
                    NodeResponse::Error("no matching registered slot for this write".to_string())
                }
            },
            NodeRequest::Fetch { mailbox_key } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                let blobs = self
                    .store
                    .fetch_all()
                    .into_iter()
                    .map(|b| pm_proto::StoredBlob {
                        id: b.id,
                        blob: b.blob,
                        delivered: b.delivered,
                    })
                    .collect();
                NodeResponse::Blobs(blobs)
            }
            NodeRequest::Ack { mailbox_key, ids } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                self.store.ack(&ids);
                NodeResponse::Ok
            }
        }
    }
}

impl ProtocolHandler for MailboxHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;

        let request_bytes = recv
            .read_to_end(MAX_MESSAGE_SIZE)
            .await
            .map_err(std::io::Error::other)?;
        let response = match bincode::deserialize::<NodeRequest>(&request_bytes) {
            Ok(request) => self.handle(request),
            Err(e) => NodeResponse::Error(format!("malformed request: {e}")),
        };

        let response_bytes =
            bincode::serialize(&response).expect("NodeResponse serialization cannot fail");
        send.write_all(&response_bytes)
            .await
            .map_err(std::io::Error::other)?;
        send.finish()?;

        connection.closed().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn handler() -> (MailboxHandler, [u8; 32]) {
        let mailbox_key = [1u8; 32];
        (
            MailboxHandler::new(Arc::new(MailboxStore::new()), mailbox_key),
            mailbox_key,
        )
    }

    #[test]
    fn fetch_and_ack_require_the_correct_mailbox_key() {
        let (handler, _real_key) = handler();
        let wrong_key = [2u8; 32];

        assert!(matches!(
            handler.handle(NodeRequest::Fetch {
                mailbox_key: wrong_key
            }),
            NodeResponse::Error(_)
        ));
        assert!(matches!(
            handler.handle(NodeRequest::Ack {
                mailbox_key: wrong_key,
                ids: vec![]
            }),
            NodeResponse::Error(_)
        ));
        assert!(matches!(
            handler.handle(NodeRequest::RegisterSlot {
                mailbox_key: wrong_key,
                slot_hash: [0u8; 32]
            }),
            NodeResponse::Error(_)
        ));
    }

    #[test]
    fn full_register_write_fetch_ack_cycle() {
        let (handler, mailbox_key) = handler();
        let auth = [9u8; 32];
        let slot_hash: [u8; 32] = Sha256::digest(auth).into();

        assert!(matches!(
            handler.handle(NodeRequest::RegisterSlot {
                mailbox_key,
                slot_hash
            }),
            NodeResponse::Ok
        ));

        assert!(matches!(
            handler.handle(NodeRequest::Write {
                auth,
                blob: b"secret bytes".to_vec()
            }),
            NodeResponse::Ok
        ));

        let NodeResponse::Blobs(blobs) = handler.handle(NodeRequest::Fetch { mailbox_key }) else {
            panic!("expected Blobs response");
        };
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].blob, b"secret bytes");
        assert!(!blobs[0].delivered);

        let id = blobs[0].id;
        assert!(matches!(
            handler.handle(NodeRequest::Ack {
                mailbox_key,
                ids: vec![id]
            }),
            NodeResponse::Ok
        ));

        let NodeResponse::Blobs(blobs) = handler.handle(NodeRequest::Fetch { mailbox_key }) else {
            panic!("expected Blobs response");
        };
        assert!(blobs[0].delivered, "ack should mark delivered");
        assert_eq!(blobs.len(), 1, "ack should not delete, per docs/PRD.md");
    }

    #[test]
    fn write_without_registration_is_rejected() {
        let (handler, _) = handler();
        assert!(matches!(
            handler.handle(NodeRequest::Write {
                auth: [0u8; 32],
                blob: vec![]
            }),
            NodeResponse::Error(_)
        ));
    }
}
