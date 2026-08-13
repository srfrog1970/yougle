//! `pm-node`: the Server mailbox binary's library half. Single-tenant,
//! self-hosted — see `docs/PRD.md` v2.0. In-memory storage only, per M2's
//! scope; persistence is a later milestone. M7 adds an outbound
//! retry-delivery sweep (`docs/PRD.md` Flow 3's third case) on top of the
//! same accept-side endpoint.

pub mod handler;
pub mod retry_queue;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use pm_proto::NODE_ALPN;
use pm_transport::NodeClient;
use thiserror::Error;
use tokio::task::JoinHandle;

pub use handler::MailboxHandler;
pub use retry_queue::RetryQueue;
pub use store::MailboxStore;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("endpoint setup failed: {0}")]
    Endpoint(String),
}

/// How often the M7 retry-delivery sweep wakes to check for due entries.
/// See `retry_queue`'s own doc comments for why this and the backoff
/// schedule it drives are small, fast-for-testing placeholder values, not
/// tuned production ones.
const SWEEP_INTERVAL: Duration = Duration::from_secs(2);

/// Per-attempt dial timeout during a retry sweep — shorter than
/// `pm-core`'s own client-facing direct-delivery timeout (20s), so one
/// unreachable recipient doesn't hold up the rest of a sweep tick for long.
const RETRY_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// A running `pm-node` instance: its accept-side [`Router`], the mailbox
/// storage backing it, the M7 retry-delivery queue, and the background
/// task sweeping that queue — bundled together so a caller (a test,
/// `main.rs`) has one thing to hold onto and one place to tear it all
/// down (see [`Self::shutdown`]).
pub struct RunningNode {
    pub router: Router,
    pub store: Arc<MailboxStore>,
    pub retry_queue: Arc<RetryQueue>,
    retry_task: JoinHandle<()>,
}

impl RunningNode {
    /// A clone of the underlying endpoint — see `Router::endpoint`'s docs.
    pub fn endpoint(&self) -> Endpoint {
        self.router.endpoint().clone()
    }

    /// Stops the retry-delivery sweep, then the accept-side `Router`.
    pub async fn shutdown(self) -> Result<(), NodeError> {
        self.retry_task.abort();
        self.router
            .shutdown()
            .await
            .map_err(|e| NodeError::Endpoint(e.to_string()))
    }
}

/// Binds an endpoint at the identity `transport_key` deterministically
/// produces (so this node's address is the same across restarts — see
/// `app/README.md`'s note on why that matters for M6's Local-delivery
/// path, which needs to dial known-in-advance identities, and for the
/// "paste this address once" Server-mailbox setup flow M5 already
/// shipped), registers the mailbox protocol handler for
/// [`pm_proto::NODE_ALPN`], starts the M7 retry-delivery sweep around a
/// clone of that same endpoint (reusing it rather than standing up a
/// second, differently-identified one just for outbound dialing — the
/// same pattern `pm-core::Client` already uses to share one endpoint
/// between its own inbound `Router` and outbound `NodeClient`), and
/// returns the resulting [`RunningNode`].
pub async fn spawn(
    mailbox_key: [u8; 32],
    transport_key: [u8; 32],
) -> Result<RunningNode, NodeError> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::from_bytes(&transport_key))
        .bind()
        .await
        .map_err(|e| NodeError::Endpoint(e.to_string()))?;

    let store = Arc::new(MailboxStore::new());
    let retry_queue = Arc::new(RetryQueue::new());
    let handler = MailboxHandler::new(store.clone(), retry_queue.clone(), mailbox_key);
    let router = Router::builder(endpoint.clone())
        .accept(NODE_ALPN, handler)
        .spawn();

    let dial_client = NodeClient::from_endpoint(endpoint);
    let retry_task = tokio::spawn(retry_sweep_loop(retry_queue.clone(), dial_client));

    Ok(RunningNode {
        router,
        store,
        retry_queue,
        retry_task,
    })
}

/// Wakes every [`SWEEP_INTERVAL`], and for each due entry spawns a
/// fire-and-forget dial attempt — matching the same best-effort
/// background-task pattern `pm-core::client`'s receipt sending already
/// uses — so one slow or unreachable recipient doesn't block the rest of
/// the sweep.
async fn retry_sweep_loop(queue: Arc<RetryQueue>, dial_client: NodeClient) {
    let mut interval = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        interval.tick().await;
        for due in queue.take_due() {
            let queue = Arc::clone(&queue);
            let dial_client = dial_client.clone();
            tokio::spawn(async move {
                let Ok(endpoint_id) = EndpointId::from_bytes(&due.recipient_transport_key) else {
                    queue.mark_failed(due.msg_id);
                    return;
                };
                let peer_addr = EndpointAddr::from(endpoint_id);
                let outcome = tokio::time::timeout(
                    RETRY_DIAL_TIMEOUT,
                    dial_client.call_direct(peer_addr, due.envelope),
                )
                .await;
                match outcome {
                    Ok(Ok(())) => queue.mark_delivered(due.msg_id),
                    _ => queue.mark_failed(due.msg_id),
                }
            });
        }
    }
}
