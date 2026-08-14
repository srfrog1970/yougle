//! One half of a two-process NAT-traversal verification: two instances of
//! this binary, each run as `--role a`/`--role b` (typically inside a
//! separate simulated-NAT network namespace — see
//! `deploy/nat-sim/run.sh`), pair for real and each send one message to
//! the other via `pm_core::Client`'s real Local-to-local direct-delivery
//! path — the exact mechanism `m6_exit_criteria.rs`'s
//! `local_only_clients_deliver_directly_with_no_server_at_all` already
//! covers, just now (when run under `run.sh`) over a real non-shared-subnet
//! path instead of loopback.
//!
//! Coordination is via plain files in a shared directory — network
//! namespaces don't isolate the filesystem, only mount namespaces do, so
//! two processes in different netns on the same host can read/write
//! ordinary files with no bind-mounts needed. Each role writes its own
//! `PairingPayload` to `<coord_dir>/<role>.tmp` then renames it to
//! `<role>.json` (atomic — a poller can never observe a partial write),
//! and polls for the other role's file to appear.
//!
//! Usage: `nat_sim_peer --role a|b [--coord-dir DIR] [--store PATH]`.
//! Prints one `RESULT role=<a|b> sent=ok|err delivered=ok|err
//! elapsed_ms=N` line per direction; exits nonzero on any failure.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pm_core::{Client, PairingPayload};
use pm_crypto::Seed;
use pm_store::MessageStatus;

const PAIRING_DEADLINE: Duration = Duration::from_secs(30);
const DELIVERY_DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    A,
    B,
}

impl Role {
    fn other(self) -> Self {
        match self {
            Role::A => Role::B,
            Role::B => Role::A,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Role::A => "a",
            Role::B => "b",
        }
    }
}

fn parse_args() -> (Role, PathBuf, PathBuf) {
    let args: Vec<String> = std::env::args().collect();
    let mut role = None;
    let mut coord_dir = PathBuf::from("/tmp/nat-sim-coord");
    let mut store = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--role" => {
                role = Some(match args[i + 1].as_str() {
                    "a" => Role::A,
                    "b" => Role::B,
                    other => panic!("--role must be 'a' or 'b', got {other:?}"),
                });
                i += 2;
            }
            "--coord-dir" => {
                coord_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--store" => {
                store = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            other => panic!("unrecognized argument {other:?}"),
        }
    }

    let role = role.expect("--role a|b is required");
    let store = store.unwrap_or_else(|| coord_dir.join(format!("{}.sqlite", role.as_str())));
    (role, coord_dir, store)
}

/// Writes `payload` to `<dir>/<role>.tmp` then renames it to
/// `<dir>/<role>.json` — the rename is atomic on the same filesystem, so a
/// concurrent poller of `read_peer_payload` can never observe a
/// partially-written file.
fn write_own_payload(dir: &Path, role: Role, payload: &PairingPayload) {
    let bytes = bincode::serialize(payload).expect("PairingPayload serialization cannot fail");
    let tmp = dir.join(format!("{}.tmp", role.as_str()));
    let dest = dir.join(format!("{}.json", role.as_str()));
    std::fs::write(&tmp, &bytes).expect("failed to write coordination file");
    std::fs::rename(&tmp, &dest).expect("failed to publish coordination file");
}

async fn read_peer_payload(dir: &Path, role: Role) -> PairingPayload {
    let path = dir.join(format!("{}.json", role.as_str()));
    let deadline = Instant::now() + PAIRING_DEADLINE;
    loop {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(payload) = bincode::deserialize(&bytes) {
                return payload;
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for peer's coordination file at {}",
                path.display()
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn print_result(role: Role, field: &str, ok: bool, elapsed: Duration) {
    println!(
        "RESULT role={} {field}={} elapsed_ms={}",
        role.as_str(),
        if ok { "ok" } else { "err" },
        elapsed.as_millis()
    );
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (role, coord_dir, store_path) = parse_args();
    std::fs::create_dir_all(&coord_dir).expect("failed to create coordination directory");

    let (seed, _) = Seed::generate();
    let client = Client::open(&seed, &store_path)
        .await
        .expect("Client::open failed");

    // --- Mutual pairing, coordinated via the shared directory. Purely
    // local filesystem I/O — proves nothing about network reachability,
    // only sets up the contact both sides need before the real thing
    // under test (send, below) can happen. ---
    let my_payload = client.pairing_payload().expect("pairing_payload failed");
    let my_nonce = my_payload.nonce;
    write_own_payload(&coord_dir, role, &my_payload);

    let their_payload = read_peer_payload(&coord_dir, role.other()).await;
    let contact_id = client
        .add_contact_from_payload(their_payload, my_nonce, Some(role.other().as_str()))
        .await
        .expect("add_contact_from_payload failed");

    // --- Barrier: wait for the *other* role to have also finished its own
    // add_contact_from_payload before either side sends anything. Without
    // this, one role can call send() while the other hasn't registered it
    // as a contact yet (the two pairing calls happen in parallel across
    // two processes, unlike m6_exit_criteria.rs's single-process test
    // where they're sequential) — the receiving side then can't attribute
    // the incoming message and rejects it. This isn't a real product bug,
    // just a race in this two-process harness. ---
    std::fs::write(coord_dir.join(format!("{}.paired", role.as_str())), b"")
        .expect("failed to write pairing-complete marker");
    let paired_marker = coord_dir.join(format!("{}.paired", role.other().as_str()));
    let deadline = Instant::now() + PAIRING_DEADLINE;
    while !paired_marker.exists() {
        if Instant::now() >= deadline {
            panic!("timed out waiting for peer to finish pairing");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    // --- The actual thing under test: one message each way. Sequential,
    // not simultaneous: Olm has exactly one session per pair (see
    // `ClientShared::add_contact`'s own doc comment), established lazily
    // by whichever side sends *first* — a second, independent
    // `create_outbound_session` from the other side before it has ever
    // received anything would produce a second, mismatched session for
    // the same pair instead of reusing the one already established via
    // `accept_incoming`. Real usage never hits this (a reply always comes
    // after receiving something first); `m6_exit_criteria.rs`'s own test
    // sends Alice-then-Bob for the same reason. So: role A sends and
    // confirms delivery first; role B waits to *receive* A's message
    // (establishing B's side of the one shared session) before sending
    // its own reply, which A then waits to receive.
    match role {
        Role::A => {
            send_and_confirm(&client, contact_id, role).await;
            wait_for_incoming(&client, contact_id, role).await;
        }
        Role::B => {
            wait_for_incoming(&client, contact_id, role).await;
            send_and_confirm(&client, contact_id, role).await;
        }
    }
}

/// Sends one message to `contact_id` and confirms it, printing a `RESULT
/// ... sent=...` line. A direct send confirms synchronously — by the time
/// `send()` returns `Ok`, this device's own outgoing copy should already
/// show `Delivered`, per `m6_exit_criteria.rs`'s own comment on this exact
/// behavior — so no polling needed here, unlike `wait_for_incoming`.
async fn send_and_confirm(client: &Client, contact_id: i64, role: Role) {
    let message = format!("hello from {}", role.as_str());
    let send_start = Instant::now();
    let send_result = client.send(contact_id, message.as_bytes()).await;
    print_result(role, "sent", send_result.is_ok(), send_start.elapsed());
    if let Err(e) = send_result {
        eprintln!("send failed: {e}");
        std::process::exit(1);
    }

    let history = client
        .messages_for_contact(contact_id)
        .expect("reading message history cannot fail");
    let status = history.iter().rev().find_map(|m| m.status);
    if status != Some(MessageStatus::Delivered) {
        eprintln!("warning: own outgoing message status is {status:?}, expected Some(Delivered)");
    }
}

/// Polls until an incoming message (one with no `status`, per
/// `pm_store::StoredMessage`'s convention) shows up for `contact_id`,
/// printing a `RESULT ... delivered=...` line. Genuinely async — depends
/// on the peer process — hence a bounded poll rather than an immediate
/// check, matching `m6_exit_criteria.rs`'s own polling idiom.
async fn wait_for_incoming(client: &Client, contact_id: i64, role: Role) {
    let start = Instant::now();
    let deadline = start + DELIVERY_DEADLINE;
    let mut delivered = false;
    while Instant::now() < deadline {
        let history = client
            .messages_for_contact(contact_id)
            .expect("reading message history cannot fail");
        if history.iter().any(|m| m.status.is_none()) {
            delivered = true;
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    print_result(role, "delivered", delivered, start.elapsed());
    if !delivered {
        std::process::exit(1);
    }
}
