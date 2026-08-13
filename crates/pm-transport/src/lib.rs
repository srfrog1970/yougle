//! `pm-transport`: iroh endpoint setup and the client side of talking to a
//! Server mailbox (`pm-node`) over the wire protocol defined in
//! `pm_proto::node_protocol`. This is a stand-in for what `pm-core` will
//! eventually own — `pm-core` doesn't exist yet (see `docs/PRD.md`'s
//! milestone plan), so M2's integration test uses this crate directly.

pub mod error;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use pm_proto::{NodeRequest, NodeResponse, MAX_MESSAGE_SIZE, NODE_ALPN};

use error::Result;
pub use error::TransportError;

/// A client endpoint for dialing out to Server mailboxes. One instance is
/// enough for a whole app session; a fresh QUIC connection is opened per
/// call (see `docs/PRD.md`'s Open Items — connection reuse/multiplexing is
/// a later efficiency concern, not a v0 requirement).
pub struct NodeClient {
    endpoint: Endpoint,
}

impl NodeClient {
    pub async fn new() -> Result<Self> {
        let endpoint = Endpoint::bind(presets::N0)
            .await
            .map_err(|e| TransportError::Endpoint(e.to_string()))?;
        Ok(Self { endpoint })
    }

    /// This endpoint's own address, shareable with a peer so they can dial
    /// back (used by the M2 test to have the "client" also be reachable,
    /// though `NodeClient` itself has no accept-side logic — that's
    /// `pm-node`'s job).
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Waits until this endpoint knows how it's network-reachable. Not
    /// required before `call` (dialing out doesn't need it), but useful
    /// before sharing `addr()` with a peer who needs to dial in.
    pub async fn online(&self) {
        self.endpoint.online().await;
    }

    /// Sends one request to the mailbox at `node_addr` and returns its
    /// response.
    pub async fn call(
        &self,
        node_addr: EndpointAddr,
        request: &NodeRequest,
    ) -> Result<NodeResponse> {
        let conn = self
            .endpoint
            .connect(node_addr, NODE_ALPN)
            .await
            .map_err(|e| TransportError::Connect(e.to_string()))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;

        let request_bytes =
            bincode::serialize(request).map_err(|e| TransportError::Codec(e.to_string()))?;
        send.write_all(&request_bytes)
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;
        send.finish()
            .map_err(|e| TransportError::Stream(e.to_string()))?;

        let response_bytes = recv
            .read_to_end(MAX_MESSAGE_SIZE)
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;
        let response = bincode::deserialize(&response_bytes)
            .map_err(|e| TransportError::Codec(e.to_string()))?;

        conn.close(0u32.into(), b"done");
        Ok(response)
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}
