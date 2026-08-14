//! Real, network-driven proof that the node protocol's per-identity rate
//! limiter (`crates/pm-node/src/rate_limit.rs`) actually gates real QUIC
//! connections, not just the in-memory `RateLimiter` unit tests — spawns a
//! real node with its real default threshold, hammers it from one real
//! `NodeClient` past that threshold, and confirms a second, independent
//! client is unaffected.

use std::time::Duration;

use pm_crypto::Identity;
use pm_crypto::Seed;
use pm_proto::{NodeRequest, NodeResponse};
use pm_transport::NodeClient;

// Matches pm_node::lib.rs's real RATE_LIMIT_MAX_PER_WINDOW — not exported,
// so duplicated here; a mismatch would just make this test loop a
// different number of times, not silently pass.
const RATE_LIMIT_MAX_PER_WINDOW: u32 = 60;

/// `RegisterSlot { mailbox_key: [0u8; 32], .. }` is rejected by ownership
/// regardless of rate-limiting (the node's real owner key is never all
/// zero bytes) — that's a *different* `NodeResponse::Error(_)` from a
/// throttled one, so the two must be told apart by message content, not
/// just by variant.
fn is_rate_limited(response: &NodeResponse) -> bool {
    matches!(response, NodeResponse::Error(msg) if msg.contains("rate limit"))
}

fn is_ownership_rejection(response: &NodeResponse) -> bool {
    matches!(response, NodeResponse::Error(msg) if msg.contains("mailbox owner") || msg.contains("no matching registered slot"))
}

#[tokio::test]
async fn a_flooding_identity_is_throttled_while_another_is_unaffected() {
    let (owner_seed, _) = Seed::generate();
    let owner_identity = Identity::derive(&owner_seed);

    let node = pm_node::spawn(
        owner_identity.mailbox_key,
        owner_identity.server_transport_key,
        None,
    )
    .await
    .expect("node starts");
    node.endpoint().online().await;
    let node_addr = node.endpoint().addr();

    let (flooder_seed, _) = Seed::generate();
    let flooder_identity = Identity::derive(&flooder_seed);
    let flooder = NodeClient::new(flooder_identity.transport_key)
        .await
        .expect("flooder endpoint binds");

    // RegisterSlot/Write are the only rate-limited request types (see
    // rate_limit.rs's module doc for why Fetch/Ack/PollFailedDeliveries
    // are deliberately excluded) — a bogus slot_hash, so every *unthrottled*
    // attempt is rejected by ownership regardless; what's under test is
    // whether the request even reaches that check.
    let harmless_request = NodeRequest::RegisterSlot {
        mailbox_key: [0u8; 32],
        slot_hash: [0u8; 32],
    };

    let mut ownership_rejections = 0u32;
    let mut throttled_count = 0u32;
    for _ in 0..(RATE_LIMIT_MAX_PER_WINDOW + 5) {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            flooder.call(node_addr.clone(), &harmless_request),
        )
        .await
        .expect("call should not hang — the node always responds, even when throttling")
        .expect("call succeeds at the transport level");

        if is_rate_limited(&response) {
            throttled_count += 1;
        } else if is_ownership_rejection(&response) {
            ownership_rejections += 1;
        } else {
            panic!("unexpected response: {response:?}");
        }
    }

    assert!(
        throttled_count > 0,
        "expected at least one throttled response; got {ownership_rejections} ownership rejections and 0 rate-limited"
    );
    assert_eq!(
        ownership_rejections, RATE_LIMIT_MAX_PER_WINDOW,
        "exactly the configured cap's worth of requests should reach the ownership check before throttling kicks in"
    );

    // A different identity's budget is untouched by the flooder's.
    let (other_seed, _) = Seed::generate();
    let other_identity = Identity::derive(&other_seed);
    let other = NodeClient::new(other_identity.transport_key)
        .await
        .expect("second endpoint binds");
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        other.call(node_addr.clone(), &harmless_request),
    )
    .await
    .expect("a fresh identity's request must not be throttled by someone else's flood")
    .expect("call succeeds");
    assert!(
        is_ownership_rejection(&response),
        "a fresh identity's first request should reach the ownership check, not be throttled: {response:?}"
    );

    flooder.close().await;
    other.close().await;
    node.shutdown().await.unwrap();
}
