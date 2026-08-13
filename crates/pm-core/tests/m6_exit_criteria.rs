//! M6 exit criteria, per `docs/PRD.md`'s milestone plan: "Local-to-local
//! live delivery and a true 'delivered' receipt."
//!
//! Covers: two Local-only clients (no Server mailbox at all) exchanging
//! messages via direct P2P delivery; a Server-mailbox conversation's status
//! transitioning from `Sent` to `Delivered` once the recipient's receipt
//! round-trips back; a direct-delivery attempt to an unreachable peer
//! failing within its bounded timeout and persisting nothing; and
//! `pm-node`'s address being stable across restarts (the M6 fix that made
//! Local delivery possible at all — see `pm-core::client`'s module doc).

use std::time::Duration;

use pm_core::Client;
use pm_crypto::Seed;
use pm_store::MessageStatus;
use tempfile::tempdir;

#[tokio::test]
async fn local_only_clients_deliver_directly_with_no_server_at_all() {
    let dir = tempdir().unwrap();

    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();

    let alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    let bob = Client::open(&bob_seed, &dir.path().join("bob.sqlite"))
        .await
        .unwrap();

    // Real mutual pairing, neither side has a Server mailbox configured —
    // both `PairingPayload.server_addr` are `None`, so every send below
    // must go via direct P2P or not at all.
    let alice_payload = alice.pairing_payload().unwrap();
    let bob_payload = bob.pairing_payload().unwrap();
    let alice_nonce = alice_payload.nonce;
    let bob_nonce = bob_payload.nonce;

    let bob_contact_id = alice
        .add_contact_from_payload(bob_payload, alice_nonce, Some("Bob"))
        .await
        .unwrap();
    let alice_contact_id = bob
        .add_contact_from_payload(alice_payload, bob_nonce, Some("Alice"))
        .await
        .unwrap();

    // A direct delivery is synchronous confirmation: the sender's own copy
    // is already `Delivered`, no receipt round-trip required for this.
    alice
        .send(bob_contact_id, b"hi bob, local only")
        .await
        .unwrap();
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history.len(), 1);
    assert_eq!(alice_history[0].status, Some(MessageStatus::Delivered));

    // And it actually arrived — by the time `send` returned, Bob's own
    // accept handler had already run (the ack is only sent back once
    // handling completes).
    let bob_history = bob.messages_for_contact(alice_contact_id).unwrap();
    assert_eq!(bob_history.len(), 1);
    assert_eq!(bob_history[0].plaintext, b"hi bob, local only");
    assert_eq!(bob_history[0].status, None); // incoming messages have no status

    // Works the other direction too.
    bob.send(alice_contact_id, b"hi alice, replying")
        .await
        .unwrap();
    let bob_history = bob.messages_for_contact(alice_contact_id).unwrap();
    assert_eq!(bob_history[1].status, Some(MessageStatus::Delivered));
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history.len(), 2);
    assert_eq!(alice_history[1].plaintext, b"hi alice, replying");
}

#[tokio::test]
async fn server_mailbox_status_transitions_from_sent_to_delivered_via_receipt() {
    let dir = tempdir().unwrap();

    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = pm_crypto::Identity::derive(&alice_seed);
    let bob_identity = pm_crypto::Identity::derive(&bob_seed);

    let alice_node = pm_node::spawn(
        alice_identity.mailbox_key,
        alice_identity.server_transport_key,
    )
    .await
    .unwrap();
    let bob_node = pm_node::spawn(bob_identity.mailbox_key, bob_identity.server_transport_key)
        .await
        .unwrap();
    alice_node.endpoint().online().await;
    bob_node.endpoint().online().await;
    let alice_server_addr = alice_node.endpoint().addr();
    let bob_server_addr = bob_node.endpoint().addr();

    let alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    alice.set_own_server_addr(alice_server_addr).unwrap();
    let bob = Client::open(&bob_seed, &dir.path().join("bob.sqlite"))
        .await
        .unwrap();
    bob.set_own_server_addr(bob_server_addr).unwrap();

    let alice_payload = alice.pairing_payload().unwrap();
    let bob_payload = bob.pairing_payload().unwrap();
    let alice_nonce = alice_payload.nonce;
    let bob_nonce = bob_payload.nonce;

    let bob_contact_id = alice
        .add_contact_from_payload(bob_payload, alice_nonce, Some("Bob"))
        .await
        .unwrap();
    let _alice_contact_id = bob
        .add_contact_from_payload(alice_payload, bob_nonce, Some("Alice"))
        .await
        .unwrap();

    // Written to Bob's mailbox — status starts as `Sent`, not `Delivered`
    // (nothing confirms receipt yet).
    alice
        .send(bob_contact_id, b"hey bob (mailbox path)")
        .await
        .unwrap();
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history[0].status, Some(MessageStatus::Sent));

    // Bob fetches it, which also spawns a best-effort receipt back to
    // Alice's own mailbox (Alice has one on file, so it goes via Server
    // too, not direct).
    let processed = bob.sync().await.unwrap();
    assert_eq!(processed, 1);

    // The receipt send is a background task, not awaited by `sync` — poll
    // rather than sleep-and-hope for it to land and for Alice's own sync
    // to pick it up.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    while tokio::time::Instant::now() < deadline {
        alice.sync().await.unwrap();
        let history = alice.messages_for_contact(bob_contact_id).unwrap();
        if history[0].status == Some(MessageStatus::Delivered) {
            delivered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        delivered,
        "message status never transitioned from Sent to Delivered"
    );

    alice_node.shutdown().await.unwrap();
    bob_node.shutdown().await.unwrap();
}

#[tokio::test]
async fn direct_delivery_to_an_unreachable_peer_times_out_and_persists_nothing() {
    let dir = tempdir().unwrap();
    let (alice_seed, _) = Seed::generate();
    let alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();

    // A "ghost" peer: real keys, but never bound to any running endpoint —
    // nothing is listening on this transport key, anywhere. Built by hand
    // (not via a real `Client`/`pairing_payload`) specifically so opening
    // it never spins up a live accept loop that would make it reachable.
    let (ghost_seed, _) = Seed::generate();
    let ghost_identity = pm_crypto::Identity::derive(&ghost_seed);
    let mut ghost_account = pm_crypto::MyAccount::new();
    let ghost_curve25519_key = ghost_account.curve25519_key().to_bytes();
    let ghost_otk = *ghost_account
        .generate_one_time_keys(1)
        .values()
        .next()
        .unwrap()
        .as_bytes();

    let ghost_contact_id = alice
        .add_contact(
            ghost_identity.signing_key.verifying_key().to_bytes(),
            ghost_curve25519_key,
            ghost_otk,
            ghost_identity.transport_key,
            Some("Ghost"),
            None, // no Server mailbox on file -> forces the direct path
            [9u8; 32],
        )
        .await
        .unwrap();

    let started = tokio::time::Instant::now();
    let result = alice.send(ghost_contact_id, b"into the void").await;
    let elapsed = started.elapsed();

    assert!(result.is_err(), "sending to an unreachable peer must fail");
    assert!(
        elapsed < Duration::from_secs(25),
        "direct delivery must give up within its bounded timeout, took {elapsed:?}"
    );
    assert!(
        alice
            .messages_for_contact(ghost_contact_id)
            .unwrap()
            .is_empty(),
        "a failed send must not persist anything"
    );
}

#[tokio::test]
async fn pm_node_address_is_stable_across_restarts() {
    let (seed, _) = Seed::generate();
    let identity = pm_crypto::Identity::derive(&seed);

    let node1 = pm_node::spawn(identity.mailbox_key, identity.server_transport_key)
        .await
        .unwrap();
    node1.endpoint().online().await;
    let id1 = node1.endpoint().addr().id;
    node1.shutdown().await.unwrap();

    let node2 = pm_node::spawn(identity.mailbox_key, identity.server_transport_key)
        .await
        .unwrap();
    node2.endpoint().online().await;
    let id2 = node2.endpoint().addr().id;
    node2.shutdown().await.unwrap();

    assert_eq!(
        id1, id2,
        "the same mailbox/transport keys must produce the same address across restarts"
    );
}
