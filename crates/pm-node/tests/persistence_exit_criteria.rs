//! Restart-durability for `pm-node`'s SQLCipher-backed storage: mailbox
//! blobs (with their delivered state), the backup blob, and a still-pending
//! M7 retry-delivery entry all survive a real `shutdown()`/`spawn()` cycle
//! against the same `data_dir`, driven through the real wire protocol (not
//! just internal struct assertions) — modeled on `pm-store`'s own
//! `tests/m1_exit_criteria.rs` restart simulation and its wrong-key test in
//! `src/lib.rs`.

use std::time::Duration;

use pm_crypto::{Identity, Seed};
use pm_proto::{NodeRequest, NodeResponse};
use pm_transport::NodeClient;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[tokio::test]
async fn mailbox_and_retry_state_survive_a_restart_against_the_same_data_dir() {
    let dir = tempdir().unwrap();
    let (seed, _) = Seed::generate();
    let identity = Identity::derive(&seed);

    // --- First run: register a slot, write a blob, ack it, store a
    // backup, and schedule a retry-delivery entry. ---
    let node = pm_node::spawn(
        identity.mailbox_key,
        identity.server_transport_key,
        Some(dir.path()),
    )
    .await
    .expect("node starts");
    node.endpoint().online().await;
    let node_addr = node.endpoint().addr();

    let writer = NodeClient::new(identity.transport_key)
        .await
        .expect("writer endpoint binds");

    let auth = [7u8; 32];
    let slot_hash: [u8; 32] = Sha256::digest(auth).into();
    let response = writer
        .call(
            node_addr.clone(),
            &NodeRequest::RegisterSlot {
                mailbox_key: identity.mailbox_key,
                slot_hash,
            },
        )
        .await
        .expect("register call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    let response = writer
        .call(
            node_addr.clone(),
            &NodeRequest::Write {
                auth,
                blob: b"durable message".to_vec(),
            },
        )
        .await
        .expect("write call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    let NodeResponse::Blobs(blobs) = writer
        .call(
            node_addr.clone(),
            &NodeRequest::Fetch {
                mailbox_key: identity.mailbox_key,
            },
        )
        .await
        .expect("fetch call succeeds")
    else {
        panic!("expected a Blobs response");
    };
    assert_eq!(blobs.len(), 1);
    let blob_id = blobs[0].id;

    let response = writer
        .call(
            node_addr.clone(),
            &NodeRequest::Ack {
                mailbox_key: identity.mailbox_key,
                ids: vec![blob_id],
            },
        )
        .await
        .expect("ack call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    let response = writer
        .call(
            node_addr.clone(),
            &NodeRequest::PutBackup {
                mailbox_key: identity.mailbox_key,
                blob: b"encrypted backup bundle".to_vec(),
            },
        )
        .await
        .expect("put backup call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    let response = writer
        .call(
            node_addr.clone(),
            &NodeRequest::ScheduleRetry {
                mailbox_key: identity.mailbox_key,
                msg_id: [9u8; 16],
                recipient_transport_key: [8u8; 32],
                envelope: b"queued envelope".to_vec(),
            },
        )
        .await
        .expect("schedule retry call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    writer.close().await;
    node.shutdown().await.unwrap();

    // --- Second run: same data_dir, same mailbox_key — everything above
    // must still be there, with no re-registration or re-scheduling. ---
    let node = pm_node::spawn(
        identity.mailbox_key,
        identity.server_transport_key,
        Some(dir.path()),
    )
    .await
    .expect("node restarts against the same data_dir");
    node.endpoint().online().await;
    let node_addr = node.endpoint().addr();

    let reader = NodeClient::new(identity.transport_key)
        .await
        .expect("reader endpoint binds");

    let NodeResponse::Blobs(blobs) = reader
        .call(
            node_addr.clone(),
            &NodeRequest::Fetch {
                mailbox_key: identity.mailbox_key,
            },
        )
        .await
        .expect("fetch call succeeds after restart")
    else {
        panic!("expected a Blobs response");
    };
    assert_eq!(blobs.len(), 1, "the blob must survive the restart");
    assert_eq!(
        blobs[0].id, blob_id,
        "ids must stay stable across a restart"
    );
    assert_eq!(blobs[0].blob, b"durable message");
    assert!(
        blobs[0].delivered,
        "the pre-restart ack's delivered state must survive"
    );

    let NodeResponse::Backup(backup) = reader
        .call(
            node_addr.clone(),
            &NodeRequest::GetBackup {
                mailbox_key: identity.mailbox_key,
            },
        )
        .await
        .expect("get backup call succeeds after restart")
    else {
        panic!("expected a Backup response");
    };
    assert_eq!(backup, Some(b"encrypted backup bundle".to_vec()));

    // The retry entry survived too. No wire request lists *pending*
    // entries (only exhausted ones, via PollFailedDeliveries) — and
    // inspecting `node.retry_queue` directly would race the node's own
    // live sweep loop, which starts sweeping the instant it's respawned
    // (its very first `interval.tick()` resolves immediately, per
    // tokio's own semantics) and will have already claimed the entry
    // before test code gets a chance to look. So: let the real sweep loop
    // run its course against the bogus recipient key and poll for the
    // entry to surface as exhausted-and-failed — proving both that the
    // envelope/recipient data survived the restart intact (it's what the
    // sweep loop actually dials) and that the entry's lifecycle continues
    // correctly afterward, not just that a row exists. Generous deadline
    // matching `pm-core`'s own `m7_exit_criteria.rs` exhaustion test,
    // which drives the same real backoff schedule.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut failed_ids = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let NodeResponse::FailedDeliveries(ids) = reader
            .call(
                node_addr.clone(),
                &NodeRequest::PollFailedDeliveries {
                    mailbox_key: identity.mailbox_key,
                },
            )
            .await
            .expect("poll failed deliveries call succeeds")
        else {
            panic!("expected a FailedDeliveries response");
        };
        if !ids.is_empty() {
            failed_ids = ids;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        failed_ids,
        vec![[9u8; 16]],
        "the retry entry scheduled before the restart never surfaced as failed after it, \
         via the same msg_id"
    );

    // A slot consumed before the restart must not be re-usable after it —
    // proves the one-time-use consumption itself was durable, not just the
    // blob it produced.
    let response = reader
        .call(
            node_addr.clone(),
            &NodeRequest::Write {
                auth,
                blob: b"replay attempt".to_vec(),
            },
        )
        .await
        .expect("call itself succeeds, even though the write is rejected");
    assert!(matches!(response, NodeResponse::Error(_)));

    reader.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn reopening_an_existing_data_dir_with_the_wrong_mailbox_key_fails() {
    let dir = tempdir().unwrap();
    let (seed, _) = Seed::generate();
    let (other_seed, _) = Seed::generate();
    let identity = Identity::derive(&seed);
    let other_identity = Identity::derive(&other_seed);

    let node = pm_node::spawn(
        identity.mailbox_key,
        identity.server_transport_key,
        Some(dir.path()),
    )
    .await
    .expect("node starts");
    node.shutdown().await.unwrap();

    let result = pm_node::spawn(
        other_identity.mailbox_key,
        identity.server_transport_key,
        Some(dir.path()),
    )
    .await;
    assert!(
        result.is_err(),
        "opening an already-migrated data_dir with a different mailbox_key must fail, \
         proving the at-rest encryption is real rather than a no-op"
    );
}
