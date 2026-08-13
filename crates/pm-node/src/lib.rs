//! `pm-node`: the Server mailbox binary's library half. Single-tenant,
//! self-hosted — see `docs/PRD.md` v2.0. In-memory storage only, per M2's
//! scope; persistence is a later milestone.

pub mod handler;
pub mod store;

use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::Endpoint;
use pm_proto::NODE_ALPN;
use thiserror::Error;

pub use handler::MailboxHandler;
pub use store::MailboxStore;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("endpoint setup failed: {0}")]
    Endpoint(String),
}

/// Binds an endpoint, registers the mailbox protocol handler for
/// [`pm_proto::NODE_ALPN`], and returns the running [`Router`] plus the
/// [`MailboxStore`] backing it (handed back so a caller — a test, or
/// `main.rs` for diagnostics — can inspect it directly without a network
/// round trip).
pub async fn spawn(mailbox_key: [u8; 32]) -> Result<(Router, Arc<MailboxStore>), NodeError> {
    let endpoint = Endpoint::bind(presets::N0)
        .await
        .map_err(|e| NodeError::Endpoint(e.to_string()))?;
    let store = Arc::new(MailboxStore::new());
    let handler = MailboxHandler::new(store.clone(), mailbox_key);
    let router = Router::builder(endpoint).accept(NODE_ALPN, handler).spawn();
    Ok((router, store))
}
