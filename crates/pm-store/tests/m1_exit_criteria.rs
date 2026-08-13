//! M1 exit criteria, per `docs/PRD.md`'s milestone plan (adapted from
//! `ARCHIT_1.MD` §7): "two in-process identities exchange encrypted
//! messages and persist them." This ties together all three M1 crates:
//! `pm-proto` (envelope framing), `pm-crypto` (identity + Olm sessions),
//! and `pm-store` (encrypted persistence) — deliberately living here as an
//! integration test rather than inside any one crate's own unit tests,
//! since it exercises the seam between all three.

use ed25519_dalek::Signer;
use pm_crypto::{Identity, MyAccount, MySession, Seed};
use pm_proto::Envelope;
use pm_store::{Direction, MessageStatus, NewMessage, Store};
use tempfile::tempdir;

/// Round-trips an Envelope through pm-proto's padding/serialization, as it
/// would travel over the wire, to prove the three crates' framing actually
/// agrees end to end (not just that each crate's own unit tests pass).
fn through_the_wire(env: &Envelope) -> Envelope {
    let padded = env
        .to_padded_bytes()
        .expect("envelope fits a padding bucket");
    Envelope::from_padded_bytes(&padded).expect("padded envelope round-trips")
}

#[test]
fn two_identities_exchange_encrypted_messages_and_persist_them() {
    // --- Long-term identities (recoverable from a seed phrase alone) ---
    let (alice_seed, _) = Seed::generate();
    let (bob_seed, _) = Seed::generate();
    let alice_identity = Identity::derive(&alice_seed);
    let bob_identity = Identity::derive(&bob_seed);

    let alice_ik = alice_identity.signing_key.verifying_key().to_bytes();
    let bob_ik = bob_identity.signing_key.verifying_key().to_bytes();

    // --- vodozemac accounts/sessions (randomly generated, not seed-derived) ---
    let alice_account = MyAccount::new();
    let mut bob_account = MyAccount::new();
    let bob_otk = *bob_account
        .generate_one_time_keys(1)
        .values()
        .next()
        .unwrap();

    let mut alice_session = alice_account
        .create_outbound_session(bob_account.curve25519_key(), bob_otk)
        .expect("alice can start a session to bob");

    // --- Each side's own encrypted local store ---
    let dir = tempdir().unwrap();
    let alice_store = Store::open(
        &dir.path().join("alice.sqlite"),
        &alice_identity.mailbox_key,
    )
    .expect("alice's store opens");
    let bob_store = Store::open(&dir.path().join("bob.sqlite"), &bob_identity.mailbox_key)
        .expect("bob's store opens");

    let bob_contact_id = alice_store
        .upsert_contact(
            &bob_ik,
            &bob_account.curve25519_key().to_bytes(),
            Some("Bob"),
        )
        .unwrap();
    let alice_contact_id = bob_store
        .upsert_contact(
            &alice_ik,
            &alice_account.curve25519_key().to_bytes(),
            Some("Alice"),
        )
        .unwrap();

    // --- Alice sends the first message (a pre-key message, since bob has no session yet) ---
    let plaintext_1 = b"hello bob, this is alice";
    let (olm_type, ciphertext) = alice_session.encrypt(plaintext_1).unwrap();
    let sig = alice_identity.signing_key.sign(&ciphertext);
    let envelope = through_the_wire(&Envelope::new(
        olm_type,
        ciphertext,
        0,
        0,
        1_754_000_000_000,
        [1u8; 16],
    ));
    // The signature isn't part of the envelope itself (that's pairing/pointer-record
    // territory per docs/PRD.md §8), but confirms IK is usable end to end here too.
    assert!(alice_identity
        .signing_key
        .verifying_key()
        .verify_strict(&envelope.ciphertext, &sig)
        .is_ok());

    let (mut bob_session, decrypted_1) = bob_account
        .accept_incoming(
            alice_account.curve25519_key(),
            envelope.olm_type,
            &envelope.ciphertext,
        )
        .expect("bob establishes an inbound session from alice's pre-key message");
    assert_eq!(decrypted_1, plaintext_1);

    // --- Persist message 1 on both sides ---
    let alice_lamport = alice_store.tick_lamport().unwrap();
    alice_store
        .insert_message(
            bob_contact_id,
            NewMessage {
                msg_id: envelope.msg_id,
                direction: Direction::Outgoing,
                lamport: alice_lamport,
                sent_at: envelope.sent_at,
                plaintext: plaintext_1,
                status: Some(MessageStatus::Sent),
            },
        )
        .unwrap();

    let bob_lamport = bob_store.observe_lamport(alice_lamport).unwrap();
    bob_store
        .insert_message(
            alice_contact_id,
            NewMessage {
                msg_id: envelope.msg_id,
                direction: Direction::Incoming,
                lamport: bob_lamport,
                sent_at: envelope.sent_at,
                plaintext: &decrypted_1,
                status: None,
            },
        )
        .unwrap();

    // --- Bob replies (now a Normal message, since the session is established) ---
    let plaintext_2 = b"hi alice, bob here";
    let (olm_type, ciphertext) = bob_session.encrypt(plaintext_2).unwrap();
    let envelope_2 = through_the_wire(&Envelope::new(
        olm_type,
        ciphertext,
        0,
        bob_lamport,
        1_754_000_001_000,
        [2u8; 16],
    ));
    let decrypted_2 = alice_session
        .decrypt(envelope_2.olm_type, &envelope_2.ciphertext)
        .unwrap();
    assert_eq!(decrypted_2, plaintext_2);

    let bob_lamport_2 = bob_store.tick_lamport().unwrap();
    bob_store
        .insert_message(
            alice_contact_id,
            NewMessage {
                msg_id: envelope_2.msg_id,
                direction: Direction::Outgoing,
                lamport: bob_lamport_2,
                sent_at: envelope_2.sent_at,
                plaintext: plaintext_2,
                status: Some(MessageStatus::Sent),
            },
        )
        .unwrap();
    let alice_lamport_2 = alice_store.observe_lamport(bob_lamport_2).unwrap();
    alice_store
        .insert_message(
            bob_contact_id,
            NewMessage {
                msg_id: envelope_2.msg_id,
                direction: Direction::Incoming,
                lamport: alice_lamport_2,
                sent_at: envelope_2.sent_at,
                plaintext: &decrypted_2,
                status: None,
            },
        )
        .unwrap();

    // --- Persist session state on both sides ---
    alice_store
        .save_session_pickle(bob_contact_id, &alice_session.pickle())
        .unwrap();
    bob_store
        .save_session_pickle(alice_contact_id, &bob_session.pickle())
        .unwrap();

    // --- Simulate an app restart: drop the in-memory session objects and
    // stores entirely, then reload everything from disk. ---
    drop(alice_session);
    drop(bob_session);
    drop(alice_store);
    drop(bob_store);

    let alice_store = Store::open(
        &dir.path().join("alice.sqlite"),
        &alice_identity.mailbox_key,
    )
    .unwrap();
    let bob_store = Store::open(&dir.path().join("bob.sqlite"), &bob_identity.mailbox_key).unwrap();

    // Message history survived, in Lamport order.
    let alice_messages = alice_store.messages_for_contact(bob_contact_id).unwrap();
    assert_eq!(alice_messages.len(), 2);
    assert_eq!(alice_messages[0].plaintext, plaintext_1);
    assert_eq!(alice_messages[1].plaintext, plaintext_2);

    let bob_messages = bob_store.messages_for_contact(alice_contact_id).unwrap();
    assert_eq!(bob_messages.len(), 2);
    assert_eq!(bob_messages[0].plaintext, plaintext_1);
    assert_eq!(bob_messages[1].plaintext, plaintext_2);

    // And the session itself survived: restore it and prove it's still
    // usable for a brand new message in each direction.
    let alice_pickle = alice_store
        .load_session_pickle(bob_contact_id)
        .unwrap()
        .unwrap();
    let bob_pickle = bob_store
        .load_session_pickle(alice_contact_id)
        .unwrap()
        .unwrap();
    let mut alice_session = MySession::from_pickle(&alice_pickle).unwrap();
    let mut bob_session = MySession::from_pickle(&bob_pickle).unwrap();

    let (olm_type, ciphertext) = alice_session
        .encrypt(b"still there after restart?")
        .unwrap();
    let reply = bob_session.decrypt(olm_type, &ciphertext).unwrap();
    assert_eq!(reply, b"still there after restart?");
}
