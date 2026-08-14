//! M7 exit criteria, per `docs/PRD.md` Flow 3's third delivery case: "If
//! the recipient is Local-only but the sender has their own Server
//! mailbox, the sender's client hands the message to their server, which
//! attempts delivery to the recipient's phone on a configurable retry
//! schedule... If the schedule is exhausted, the message stays in the
//! thread marked 'failed to deliver.'"
//!
//! Both tests need Bob to be a *real* paired contact who later becomes
//! unreachable, not a synthetic "ghost" like M6's timeout test — this
//! recipient must actually receive and decrypt a real message once he
//! comes back. So: open a real `Client` for Bob, pair for real, then
//! `drop` it (per M6's design, dropping a `Client` drops its `Router`,
//! stopping its Local-delivery accept loop) while his identity/store
//! persist on disk — exactly "Bob's phone is off" — and reopening
//! `Client::open` at the same path later is exactly "Bob turns it back
//! on" (deterministic identity/keys from the seed, per M6).

use std::time::Duration;

use pm_core::Client;
use pm_crypto::Seed;
use pm_store::MessageStatus;
use tempfile::tempdir;

/// Sets up Alice (her own Server, spawned + reachable) and Bob
/// (Local-only), mutually paired for real, then drops Bob's `Client` so
/// he's genuinely unreachable. Returns everything the two tests need to
/// continue from there.
async fn alice_with_server_and_a_freshly_unreachable_bob(
    dir: &std::path::Path,
) -> (
    pm_node::RunningNode,
    Client,
    i64, // bob_contact_id, from alice's side
    pm_crypto::Seed,
    std::path::PathBuf, // bob's store path, to reopen him later
) {
    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = pm_crypto::Identity::derive(&alice_seed);

    let alice_node = pm_node::spawn(
        alice_identity.mailbox_key,
        alice_identity.server_transport_key,
        None,
    )
    .await
    .unwrap();
    alice_node.endpoint().online().await;
    let alice_server_addr = alice_node.endpoint().addr();

    let alice = Client::open(&alice_seed, &dir.join("alice.sqlite"))
        .await
        .unwrap();
    alice.set_own_server_addr(alice_server_addr).unwrap();

    let bob_store_path = dir.join("bob.sqlite");
    let bob = Client::open(&bob_seed, &bob_store_path).await.unwrap();

    let alice_payload = alice.pairing_payload().unwrap();
    let bob_payload = bob.pairing_payload().unwrap();
    let alice_nonce = alice_payload.nonce;
    let bob_nonce = bob_payload.nonce;

    let bob_contact_id = alice
        .add_contact_from_payload(bob_payload, alice_nonce, Some("Bob"))
        .await
        .unwrap();
    bob.add_contact_from_payload(alice_payload, bob_nonce, Some("Alice"))
        .await
        .unwrap();

    // Bob "goes offline": his accept loop stops the moment his Client is
    // dropped, but his identity/account/contacts persist on disk.
    drop(bob);

    (alice_node, alice, bob_contact_id, bob_seed, bob_store_path)
}

#[tokio::test]
async fn fallback_queues_on_senders_server_and_delivers_once_recipient_is_reachable_again() {
    let dir = tempdir().unwrap();
    let (alice_node, alice, bob_contact_id, bob_seed, bob_store_path) =
        alice_with_server_and_a_freshly_unreachable_bob(dir.path()).await;

    // Bob is unreachable, but Alice has her own Server — send must
    // succeed (not error) by queuing on it, not fail like M6's ghost-peer
    // test (which has no Server to fall back to).
    alice
        .send(bob_contact_id, b"hi bob, catch this when you're back")
        .await
        .unwrap();
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history.len(), 1);
    assert_eq!(alice_history[0].status, Some(MessageStatus::Sent));

    // Bob "turns his phone back on" — same seed, same store path, so the
    // exact same identity/keys/pending session state, and the exact same
    // (deterministic) transport key his contact record for him already
    // has on file.
    let bob = Client::open(&bob_seed, &bob_store_path).await.unwrap();

    // Alice's own node's retry sweep should find him reachable again and
    // deliver the queued envelope — poll rather than sleep-and-hope.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut bob_received = false;
    while tokio::time::Instant::now() < deadline {
        let contacts = bob.list_contacts().unwrap();
        if let Some(alice_contact) = contacts.first() {
            let history = bob.messages_for_contact(alice_contact.id).unwrap();
            if !history.is_empty() {
                assert_eq!(history[0].plaintext, b"hi bob, catch this when you're back");
                bob_received = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        bob_received,
        "Alice's own node never delivered the queued message once Bob was reachable again"
    );

    // Bob's receipt for it round-trips back to Alice's own Server (Alice
    // has one on file in Bob's contact record) — poll for the status
    // flip, same pattern M6's own Sent-to-Delivered test uses.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut delivered = false;
    while tokio::time::Instant::now() < deadline {
        alice.sync().await.unwrap();
        let history = alice.messages_for_contact(bob_contact_id).unwrap();
        if history[0].status == Some(MessageStatus::Delivered) {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        delivered,
        "message status never transitioned from Sent to Delivered after the retry succeeded"
    );

    alice_node.shutdown().await.unwrap();
}

#[tokio::test]
async fn exhausted_retry_schedule_marks_the_message_failed() {
    let dir = tempdir().unwrap();
    let (alice_node, alice, bob_contact_id, _bob_seed, _bob_store_path) =
        alice_with_server_and_a_freshly_unreachable_bob(dir.path()).await;

    // Bob never comes back this time.
    alice
        .send(bob_contact_id, b"hi bob, are you there")
        .await
        .unwrap();
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history[0].status, Some(MessageStatus::Sent));

    // Once Alice's own node exhausts its retry schedule, the next sync
    // should pick that up via PollFailedDeliveries and mark it Failed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut failed = false;
    while tokio::time::Instant::now() < deadline {
        alice.sync().await.unwrap();
        let history = alice.messages_for_contact(bob_contact_id).unwrap();
        if history[0].status == Some(MessageStatus::Failed) {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        failed,
        "message status never transitioned to Failed once the retry schedule exhausted"
    );

    alice_node.shutdown().await.unwrap();
}
