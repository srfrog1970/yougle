//! Signed mailbox-pointer updates, per `docs/PRD.md` §8: "Any update to
//! which mailbox currently represents a user... must be published as a
//! pointer record encrypted and addressed per contact pair, and signed by
//! the user's identity key (IK). A contact's client must verify this
//! signature before trusting a new mailbox address."
//!
//! Covers the real end-to-end path: two Local-only clients pair, one
//! configures (and later reverts) their own Server mailbox, and the
//! other's contact record picks up each change in turn — via the same
//! encrypted delivery channel `Chat`/`Receipt` already use, no separate
//! mechanism or wire format. Signature/staleness rejection itself is
//! covered at the unit level in `pm-core::client`'s own test module
//! (`pointer_update_is_valid`), since that's where the actual policy
//! lives — `pm-store`'s `set_contact_pointer_update` is just the
//! mechanical "apply" step, per its own doc comment.

use std::time::Duration;

use pm_core::Client;
use pm_crypto::Seed;
use tempfile::tempdir;

#[tokio::test]
async fn pointer_update_propagates_and_a_later_change_supersedes_an_earlier_one() {
    let dir = tempdir().unwrap();

    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = pm_crypto::Identity::derive(&alice_seed);

    // Alice and Bob pair while both are Local-only.
    let alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    let bob = Client::open(&bob_seed, &dir.path().join("bob.sqlite"))
        .await
        .unwrap();

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
    let _ = bob_contact_id;

    // Bob's contact record for Alice starts with nothing on file — Alice
    // was Local-only at pairing time.
    let alice_contact = |bob: &Client| {
        bob.list_contacts()
            .unwrap()
            .into_iter()
            .find(|c| c.id == alice_contact_id)
            .unwrap()
    };
    assert_eq!(alice_contact(&bob).server_addr, None);
    assert_eq!(alice_contact(&bob).pointer_updated_at, 0);

    // Alice configures a real Server mailbox — this must broadcast a
    // signed pointer update to every known contact (just Bob here),
    // best-effort in the background, so poll for it to land rather than
    // sleep-and-hope (same pattern every prior milestone's tests use).
    let alice_node = pm_node::spawn(
        alice_identity.mailbox_key,
        alice_identity.server_transport_key,
        None,
    )
    .await
    .unwrap();
    alice_node.endpoint().online().await;
    let alice_server_addr = alice_node.endpoint().addr();
    alice.set_own_server_addr(alice_server_addr).unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut picked_up = false;
    while tokio::time::Instant::now() < deadline {
        let contact = alice_contact(&bob);
        if contact.server_addr.is_some() && contact.pointer_updated_at > 0 {
            picked_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        picked_up,
        "Bob's contact record for Alice never picked up her new Server address"
    );
    let after_first_update = alice_contact(&bob).pointer_updated_at;

    // Alice reverts to Local-only — a second, later update must supersede
    // the first, not get stuck on it.
    alice.clear_own_server_addr().unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut reverted = false;
    while tokio::time::Instant::now() < deadline {
        let contact = alice_contact(&bob);
        if contact.server_addr.is_none() && contact.pointer_updated_at > after_first_update {
            reverted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        reverted,
        "Bob's contact record for Alice never picked up her reverting to Local-only"
    );

    alice_node.shutdown().await.unwrap();
}

/// The actual property that makes replacing wall-clock ordering with a
/// locally-persisted counter safe (`docs/THREAT-MODEL.md`): restoring onto
/// a new device must not reset the counter, or every contact who'd already
/// seen a higher value would permanently reject that device's future
/// pointer updates. Alice broadcasts once (Bob records her `pointer_seq`),
/// "gets a new device" via `export_backup`/`import_backup`, then
/// broadcasts again — the second update must still be accepted by Bob,
/// proving the restored device's counter picked up where it left off
/// rather than restarting at 0.
#[tokio::test]
async fn a_devices_pointer_broadcast_seq_survives_being_restored_onto_a_new_device() {
    let dir = tempdir().unwrap();

    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = pm_crypto::Identity::derive(&alice_seed);

    let alice = Client::open(&alice_seed, &dir.path().join("alice.sqlite"))
        .await
        .unwrap();
    let bob = Client::open(&bob_seed, &dir.path().join("bob.sqlite"))
        .await
        .unwrap();

    let alice_payload = alice.pairing_payload().unwrap();
    let bob_payload = bob.pairing_payload().unwrap();
    let alice_nonce = alice_payload.nonce;
    let bob_nonce = bob_payload.nonce;

    alice
        .add_contact_from_payload(bob_payload, alice_nonce, Some("Bob"))
        .await
        .unwrap();
    let alice_contact_id = bob
        .add_contact_from_payload(alice_payload, bob_nonce, Some("Alice"))
        .await
        .unwrap();

    let alice_contact = |bob: &Client| {
        bob.list_contacts()
            .unwrap()
            .into_iter()
            .find(|c| c.id == alice_contact_id)
            .unwrap()
    };

    // First broadcast, from Alice's original device.
    let alice_node = pm_node::spawn(
        alice_identity.mailbox_key,
        alice_identity.server_transport_key,
        None,
    )
    .await
    .unwrap();
    alice_node.endpoint().online().await;
    alice
        .set_own_server_addr(alice_node.endpoint().addr())
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut picked_up = false;
    while tokio::time::Instant::now() < deadline {
        if alice_contact(&bob).pointer_seq > 0 {
            picked_up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(picked_up, "Bob never picked up Alice's first broadcast");
    let seq_bob_has_on_file = alice_contact(&bob).pointer_seq;

    // Bob's own contact table updates *before* he sends his DirectAck back
    // — Alice's own session pickle only saves *after* that ack round-trips
    // to her (`deliver`'s save_session_pickle call is the very last thing
    // it does). Observing Bob's side isn't proof Alice's own delivery
    // attempt has fully wound down; without this, `export_backup` below
    // can race a real, if narrow, window and capture Alice's state from
    // just before her own session pickle was written.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Alice "gets a new device": export from the original, drop it (an
    // iroh endpoint identity, deterministic from the seed, can't be live
    // in two places at once), then import onto a fresh store path — the
    // same restore path a real device-loss recovery goes through.
    let backup_bytes = alice.export_backup().unwrap();
    drop(alice);
    let restored_alice = Client::import_backup(
        &alice_seed,
        &dir.path().join("alice-restored.sqlite"),
        &backup_bytes,
    )
    .await
    .unwrap();

    // Second broadcast, from the *restored* device. If its counter had
    // reset to 0/1 instead of resuming from where the original left off,
    // this would be silently rejected by Bob (seq no greater than what
    // he already has on file) — exactly as permanently broken as the
    // clock-skew bug this design replaces, just triggered by a restore
    // instead of a clock jump.
    restored_alice.clear_own_server_addr().unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut accepted = false;
    while tokio::time::Instant::now() < deadline {
        let contact = alice_contact(&bob);
        if contact.server_addr.is_none() && contact.pointer_seq > seq_bob_has_on_file {
            accepted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        accepted,
        "Bob rejected the restored device's broadcast — its counter must have reset instead of resuming"
    );

    alice_node.shutdown().await.unwrap();
}
