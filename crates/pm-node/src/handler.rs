//! Dispatches incoming `NodeRequest`s to a [`MailboxStore`], enforcing the
//! one mailbox-owner check every request that touches the owner's inbox
//! requires.

use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use pm_proto::{NodeRequest, NodeResponse, MAX_MESSAGE_SIZE};

use crate::retry_queue::RetryQueue;
use crate::store::{MailboxStore, WriteError};

#[derive(Debug, Clone)]
pub struct MailboxHandler {
    store: Arc<MailboxStore>,
    retry_queue: Arc<RetryQueue>,
    /// The single tenant this node serves. Requests that claim mailbox
    /// ownership (`Fetch`, `Ack`, `RegisterSlot`, `ScheduleRetry`,
    /// `PollFailedDeliveries`) must present this exact key; `Write` never
    /// does, since a sender isn't the owner.
    mailbox_key: [u8; 32],
}

impl MailboxHandler {
    pub fn new(
        store: Arc<MailboxStore>,
        retry_queue: Arc<RetryQueue>,
        mailbox_key: [u8; 32],
    ) -> Self {
        Self {
            store,
            retry_queue,
            mailbox_key,
        }
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
            NodeRequest::PutBackup { mailbox_key, blob } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                self.store.put_backup(blob);
                NodeResponse::Ok
            }
            NodeRequest::GetBackup { mailbox_key } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                NodeResponse::Backup(self.store.get_backup())
            }
            NodeRequest::ScheduleRetry {
                mailbox_key,
                msg_id,
                recipient_transport_key,
                envelope,
            } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                self.retry_queue
                    .schedule(msg_id, recipient_transport_key, envelope);
                NodeResponse::Ok
            }
            NodeRequest::PollFailedDeliveries { mailbox_key } => {
                if mailbox_key != self.mailbox_key {
                    return NodeResponse::Error("not the mailbox owner".to_string());
                }
                NodeResponse::FailedDeliveries(self.retry_queue.take_failed())
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
        let conn = crate::db::open_for_test();
        (
            MailboxHandler::new(
                Arc::new(MailboxStore::new(conn.clone())),
                Arc::new(RetryQueue::new(conn)),
                mailbox_key,
            ),
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
    fn backup_requires_the_correct_mailbox_key_and_roundtrips() {
        let (handler, mailbox_key) = handler();
        let wrong_key = [2u8; 32];

        assert!(matches!(
            handler.handle(NodeRequest::GetBackup {
                mailbox_key: wrong_key
            }),
            NodeResponse::Error(_)
        ));

        // No backup stored yet.
        assert!(matches!(
            handler.handle(NodeRequest::GetBackup { mailbox_key }),
            NodeResponse::Backup(None)
        ));

        assert!(matches!(
            handler.handle(NodeRequest::PutBackup {
                mailbox_key,
                blob: b"encrypted bundle".to_vec()
            }),
            NodeResponse::Ok
        ));

        let NodeResponse::Backup(Some(blob)) =
            handler.handle(NodeRequest::GetBackup { mailbox_key })
        else {
            panic!("expected Backup(Some(_))");
        };
        assert_eq!(blob, b"encrypted bundle");
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

    #[test]
    fn schedule_retry_and_poll_failed_deliveries_require_the_correct_mailbox_key() {
        let (handler, _real_key) = handler();
        let wrong_key = [2u8; 32];

        assert!(matches!(
            handler.handle(NodeRequest::ScheduleRetry {
                mailbox_key: wrong_key,
                msg_id: [0u8; 16],
                recipient_transport_key: [0u8; 32],
                envelope: vec![],
            }),
            NodeResponse::Error(_)
        ));
        assert!(matches!(
            handler.handle(NodeRequest::PollFailedDeliveries {
                mailbox_key: wrong_key
            }),
            NodeResponse::Error(_)
        ));
    }

    #[test]
    fn schedule_retry_queues_the_entry_for_the_owner() {
        let (handler, mailbox_key) = handler();

        assert!(matches!(
            handler.handle(NodeRequest::ScheduleRetry {
                mailbox_key,
                msg_id: [7u8; 16],
                recipient_transport_key: [8u8; 32],
                envelope: b"envelope".to_vec(),
            }),
            NodeResponse::Ok
        ));

        let due = handler.retry_queue.take_due();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].msg_id, [7u8; 16]);
    }

    #[test]
    fn poll_failed_deliveries_drains_the_owners_exhausted_entries() {
        let (handler, mailbox_key) = handler();
        handler
            .retry_queue
            .schedule([7u8; 16], [8u8; 32], b"envelope".to_vec());
        for _ in 0..crate::retry_queue::MAX_RETRY_ATTEMPTS {
            handler.retry_queue.mark_failed([7u8; 16]);
        }

        let NodeResponse::FailedDeliveries(ids) =
            handler.handle(NodeRequest::PollFailedDeliveries { mailbox_key })
        else {
            panic!("expected FailedDeliveries response");
        };
        assert_eq!(ids, vec![[7u8; 16]]);

        // Drain-on-read: a second poll returns nothing new.
        let NodeResponse::FailedDeliveries(ids) =
            handler.handle(NodeRequest::PollFailedDeliveries { mailbox_key })
        else {
            panic!("expected FailedDeliveries response");
        };
        assert!(ids.is_empty());
    }
}
