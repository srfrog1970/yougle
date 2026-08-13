//! M5 exit criteria: the Rust/FFI surface the app UI needs actually works
//! end to end — real mutual pairing (not the hand-fed keys/secret stand-in
//! M1-M4's tests used), listing contacts, local (no-Server) backup
//! export/import, and this device's own Server address persisting across
//! restarts.

use pm_core::Client;
use pm_crypto::Seed;
use tempfile::tempdir;

#[tokio::test]
async fn mutual_pairing_lands_on_the_same_secret_and_a_working_conversation() {
    let dir = tempdir().unwrap();

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

    let mut alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    alice
        .set_own_server_addr(alice_server_addr.clone())
        .unwrap();
    let mut bob = Client::open(&bob_seed, &dir.path().join("bob.sqlite"))
        .await
        .unwrap();
    bob.set_own_server_addr(bob_server_addr.clone()).unwrap();

    // --- Real mutual pairing: each produces their own shareable payload
    // (as a QR/paste code would carry), then each completes pairing from
    // the *other's* payload plus their *own* remembered nonce. Neither
    // side is handed a pre-agreed pair_secret. ---
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

    // Both sides must have landed on the identical pair_secret — verified
    // indirectly: a message written under Alice's derived write-auth must
    // be acceptable to Bob's server, which only pre-registered slots
    // derived from *its own* view of the shared secret.
    alice.send(bob_contact_id, b"hey bob").await.unwrap();
    let processed = bob.sync().await.unwrap();
    assert_eq!(processed, 1);

    bob.send(alice_contact_id, b"hey alice").await.unwrap();
    let processed = alice.sync().await.unwrap();
    assert_eq!(processed, 1);

    // list_contacts surfaces what pairing produced.
    let alice_contacts = alice.list_contacts().unwrap();
    assert_eq!(alice_contacts.len(), 1);
    assert_eq!(alice_contacts[0].display_name.as_deref(), Some("Bob"));
    assert_eq!(
        alice_contacts[0].identity_key,
        bob_identity.signing_key.verifying_key().to_bytes()
    );

    alice_router.shutdown().await.unwrap();
    bob_router.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_backup_export_and_import_roundtrip_without_any_server() {
    let dir = tempdir().unwrap();
    let (seed, _) = Seed::generate();

    let mut original = Client::open(&seed, &dir.path().join("original.sqlite"))
        .await
        .unwrap();

    // A contact and a conversation exist purely locally — no Server
    // mailbox configured on this client at all.
    let (contact_seed, _) = Seed::generate();
    let contact_identity = pm_crypto::Identity::derive(&contact_seed);
    let mut contact_client = Client::open(&contact_seed, &dir.path().join("contact.sqlite"))
        .await
        .unwrap();

    let original_payload = original.pairing_payload().unwrap();
    let contact_payload = contact_client.pairing_payload().unwrap();
    let original_nonce = original_payload.nonce;

    let contact_id = original
        .add_contact_from_payload(contact_payload, original_nonce, Some("Friend"))
        .await
        .unwrap();
    let _ = contact_identity; // only needed to derive contact_seed's identity above

    // export_backup works with no Server at all.
    let backup_bytes = original.export_backup().unwrap();

    drop(original);

    let restored = Client::import_backup(&seed, &dir.path().join("restored.sqlite"), &backup_bytes)
        .await
        .unwrap();

    let restored_contacts = restored.list_contacts().unwrap();
    assert_eq!(restored_contacts.len(), 1);
    assert_eq!(restored_contacts[0].id, contact_id);
    assert_eq!(restored_contacts[0].display_name.as_deref(), Some("Friend"));
}

#[tokio::test]
async fn own_server_addr_persists_across_close_and_reopen() {
    let dir = tempdir().unwrap();
    let (seed, _) = Seed::generate();
    let identity = pm_crypto::Identity::derive(&seed);
    let (router, _store) = pm_node::spawn(identity.mailbox_key).await.unwrap();
    router.endpoint().online().await;
    let server_addr = router.endpoint().addr();

    let store_path = dir.path().join("store.sqlite");

    {
        let mut client = Client::open(&seed, &store_path).await.unwrap();
        assert_eq!(client.own_server_addr(), None);
        client.set_own_server_addr(server_addr.clone()).unwrap();
        assert_eq!(client.own_server_addr(), Some(server_addr.clone()));
    } // client dropped — nothing kept it alive in memory

    let reopened = Client::open(&seed, &store_path).await.unwrap();
    assert_eq!(
        reopened.own_server_addr(),
        Some(server_addr),
        "own Server address must survive a close/reopen without being re-supplied"
    );

    router.shutdown().await.unwrap();
}
