//! M2 exit criteria, per `docs/PRD.md`'s milestone plan (adapted from
//! `ARCHIT_1.MD` §7): "two [instances] in separate processes exchange a
//! message through the node, with the node's logs proving it saw only tags
//! and opaque bytes."
//!
//! Two adaptations, both explained where they matter below:
//! - `pm-core` doesn't exist yet, so `pm-transport::NodeClient` plus
//!   `pm-crypto`/`pm-proto` directly stand in for it, as in M1.
//! - "Separate processes" is approximated with separate iroh `Endpoint`s
//!   (genuine independent network identities dialing real QUIC connections
//!   over loopback) rather than separate OS processes — this preserves the
//!   property the criterion is actually protecting (the node only learns
//!   what crosses the wire, nothing via shared in-process state) without
//!   the added complexity of process/IPC orchestration.

use ed25519_dalek::Signer;
use pm_crypto::{Identity, MyAccount, Seed};
use pm_proto::{Envelope, NodeRequest, NodeResponse};
use pm_transport::NodeClient;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn two_clients_exchange_a_message_through_the_node() {
    // --- Bob is the mailbox owner; Alice is a sender who has paired with him ---
    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = Identity::derive(&alice_seed);
    let bob_identity = Identity::derive(&bob_seed);

    // --- Start Bob's node ---
    let node = pm_node::spawn(bob_identity.mailbox_key, bob_identity.server_transport_key)
        .await
        .expect("node starts");
    node.endpoint().online().await;
    let node_addr = node.endpoint().addr();

    // --- Two independent client endpoints — separate network identities,
    // standing in for "separate processes" ---
    let alice_client = NodeClient::new(alice_identity.transport_key)
        .await
        .expect("alice's endpoint binds");
    let bob_client = NodeClient::new(bob_identity.transport_key)
        .await
        .expect("bob's endpoint binds");

    // --- Bob registers a write slot for Alice, using an auth value derived
    // the same way pairing would eventually derive one (pm-core doesn't
    // exist yet to own this derivation, so it's done directly here) ---
    let pair_secret = [42u8; 32]; // stand-in for a real pairing shared secret
    let auth: [u8; 32] = pm_proto::derive(&pair_secret, "auth", 0, 32)
        .expect("derivation succeeds")
        .try_into()
        .unwrap();
    let slot_hash: [u8; 32] = Sha256::digest(auth).into();

    let response = bob_client
        .call(
            node_addr.clone(),
            &NodeRequest::RegisterSlot {
                mailbox_key: bob_identity.mailbox_key,
                slot_hash,
            },
        )
        .await
        .expect("register call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    // --- Alice and Bob establish a real Olm session and Alice encrypts a
    // real message (same crypto as the M1 test) ---
    let alice_account = MyAccount::new();
    let mut bob_account = MyAccount::new();
    let bob_otk = *bob_account
        .generate_one_time_keys(1)
        .values()
        .next()
        .unwrap();
    let mut alice_session = alice_account
        .create_outbound_session(bob_account.curve25519_key(), bob_otk)
        .unwrap();

    let plaintext = b"hello bob, this is alice, over the network this time";
    let (olm_type, ciphertext) = alice_session.encrypt(plaintext).unwrap();
    let sig = alice_identity.signing_key.sign(&ciphertext);
    let envelope = Envelope::new(olm_type, ciphertext, 0, 0, 1_754_000_000_000, [1u8; 16]);
    let envelope_bytes = envelope.to_padded_bytes().unwrap();

    // --- Alice writes it to Bob's node ---
    let response = alice_client
        .call(
            node_addr.clone(),
            &NodeRequest::Write {
                auth,
                blob: envelope_bytes,
            },
        )
        .await
        .expect("write call succeeds");
    assert!(matches!(response, NodeResponse::Ok));

    // --- The node's own storage proves it saw only an opaque blob: no
    // plaintext, no sender identity, nothing but auth-gated bytes. ---
    let stored = node.store.fetch_all();
    assert_eq!(stored.len(), 1);
    assert!(
        !stored[0]
            .blob
            .windows(plaintext.len())
            .any(|w| w == plaintext.as_slice()),
        "the node's stored blob must not contain the plaintext anywhere"
    );

    // --- Bob fetches, decrypts, and verifies the signature, exactly like a
    // real client would ---
    let NodeResponse::Blobs(blobs) = bob_client
        .call(
            node_addr.clone(),
            &NodeRequest::Fetch {
                mailbox_key: bob_identity.mailbox_key,
            },
        )
        .await
        .expect("fetch call succeeds")
    else {
        panic!("expected a Blobs response");
    };
    assert_eq!(blobs.len(), 1);

    let fetched_envelope = Envelope::from_padded_bytes(&blobs[0].blob).unwrap();
    assert!(alice_identity
        .signing_key
        .verifying_key()
        .verify_strict(&fetched_envelope.ciphertext, &sig)
        .is_ok());

    let decrypted = bob_account
        .accept_incoming(
            alice_account.curve25519_key(),
            fetched_envelope.olm_type,
            &fetched_envelope.ciphertext,
        )
        .unwrap()
        .1;
    assert_eq!(decrypted, plaintext);

    // --- Bob acks; the node retains it (per docs/PRD.md), just marks delivered ---
    let response = bob_client
        .call(
            node_addr.clone(),
            &NodeRequest::Ack {
                mailbox_key: bob_identity.mailbox_key,
                ids: vec![blobs[0].id],
            },
        )
        .await
        .expect("ack call succeeds");
    assert!(matches!(response, NodeResponse::Ok));
    assert!(node.store.fetch_all()[0].delivered);
    assert_eq!(
        node.store.fetch_all().len(),
        1,
        "acked messages are retained, not deleted"
    );

    // --- A write with no matching registered slot is rejected — the node
    // won't accept arbitrary writes from a stranger ---
    let response = alice_client
        .call(
            node_addr.clone(),
            &NodeRequest::Write {
                auth: [99u8; 32],
                blob: b"uninvited".to_vec(),
            },
        )
        .await
        .expect("call itself succeeds, even though the write is rejected");
    assert!(matches!(response, NodeResponse::Error(_)));

    alice_client.close().await;
    bob_client.close().await;
    node.shutdown().await.unwrap();
}
