//! M3 exit criteria, per `docs/PRD.md`'s milestone plan (`ARCHIT_1.MD` §7):
//! "restore a fresh identity from seed phrase and resume conversations."
//!
//! Scope note: restoring here means re-supplying the Server's address
//! directly, not an automatic DHT lookup — see `pm-core::client`'s
//! module-level doc comment for why (the original architecture doc itself
//! flags DHT publishing as its one genuinely open mechanism question).

use pm_core::Client;
use pm_crypto::Seed;
use tempfile::tempdir;

#[tokio::test]
async fn restore_from_seed_phrase_resumes_a_conversation() {
    let dir = tempdir().unwrap();

    // --- Alice and Bob each run their own Server mailbox ---
    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = pm_crypto::Identity::derive(&alice_seed);
    let bob_identity = pm_crypto::Identity::derive(&bob_seed);

    let (alice_router, _alice_store) = pm_node::spawn(alice_identity.mailbox_key).await.unwrap();
    let (bob_router, _bob_store) = pm_node::spawn(bob_identity.mailbox_key).await.unwrap();
    alice_router.endpoint().online().await;
    bob_router.endpoint().online().await;
    let alice_server_addr = alice_router.endpoint().addr();
    let bob_server_addr = bob_router.endpoint().addr();

    // --- Alice and Bob's clients ---
    let mut alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    alice
        .set_own_server_addr(alice_server_addr.clone())
        .unwrap();
    let mut bob = Client::open(&bob_seed, &dir.path().join("bob-original.sqlite"))
        .await
        .unwrap();
    bob.set_own_server_addr(bob_server_addr.clone()).unwrap();

    // --- Mutual pairing stand-in: each generates a one-time key for the
    // other and calls add_contact, exactly as a real QR exchange would feed
    // into this same API. ---
    let alice_otk = alice.generate_one_time_key().unwrap();
    let bob_otk = bob.generate_one_time_key().unwrap();
    let pair_secret = [42u8; 32];

    let bob_contact_id = alice
        .add_contact(
            bob_identity.signing_key.verifying_key().to_bytes(),
            bob.curve25519_key(),
            bob_otk,
            Some("Bob"),
            Some(bob_server_addr.clone()),
            pair_secret,
        )
        .await
        .unwrap();
    let alice_contact_id = bob
        .add_contact(
            alice_identity.signing_key.verifying_key().to_bytes(),
            alice.curve25519_key(),
            alice_otk,
            Some("Alice"),
            Some(alice_server_addr.clone()),
            pair_secret,
        )
        .await
        .unwrap();

    // --- A normal conversation happens ---
    alice.send(bob_contact_id, b"hey bob").await.unwrap();
    let processed = bob.sync().await.unwrap();
    assert_eq!(processed, 1);

    bob.send(alice_contact_id, b"hey alice, good to hear from you")
        .await
        .unwrap();
    let processed = alice.sync().await.unwrap();
    assert_eq!(processed, 1);

    let bob_history_before = bob.messages_for_contact(alice_contact_id).unwrap();
    assert_eq!(bob_history_before.len(), 2);

    // --- Bob backs up to his own server ---
    bob.push_backup().await.unwrap();

    // --- Bob loses his device entirely: drop the client, and note his
    // original store file is never touched again (restore opens a
    // brand-new one, proving nothing local survived). ---
    drop(bob);

    // --- Restore on a "new device": identity from the seed phrase alone,
    // contacts and history from the backup on Bob's own server (whose
    // address Bob re-enters, per this build's scope). ---
    let mut restored_bob = Client::restore(
        &bob_seed,
        &dir.path().join("bob-restored.sqlite"),
        bob_server_addr.clone(),
    )
    .await
    .unwrap();

    // Same identity as before restoration.
    assert_eq!(
        restored_bob.identity_key(),
        bob_identity.signing_key.verifying_key().to_bytes()
    );

    // History survived the restore.
    let restored_history = restored_bob.messages_for_contact(alice_contact_id).unwrap();
    assert_eq!(restored_history, bob_history_before);

    // And the restored Bob can actually keep talking to Alice — not just
    // display old history, but continue the live conversation.
    restored_bob
        .send(alice_contact_id, b"sorry, had to get a new phone")
        .await
        .unwrap();
    let processed = alice.sync().await.unwrap();
    assert_eq!(processed, 1);
    let alice_history = alice.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_history.len(), 3);
    assert_eq!(alice_history[2].plaintext, b"sorry, had to get a new phone");

    alice_router.shutdown().await.unwrap();
    bob_router.shutdown().await.unwrap();
}
