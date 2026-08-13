//! `pm-transport`: iroh endpoint setup and the client side of talking to a
//! Server mailbox (`pm-node`) over the wire protocol defined in
//! `pm_proto::node_protocol`. This is a stand-in for what `pm-core` will
//! eventually own — `pm-core` doesn't exist yet (see `docs/PRD.md`'s
//! milestone plan), so M2's integration test uses this crate directly.

pub mod error;

use base64::Engine;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr};
use pm_proto::{NodeRequest, NodeResponse, MAX_MESSAGE_SIZE, NODE_ALPN};

use error::Result;
pub use error::TransportError;

/// Encodes an `EndpointAddr` as a string a person can paste, type, or see
/// in a QR code — `iroh-base 1.0.3` has no `Display`/`FromStr` for the full
/// struct (only for its `EndpointId`/`TransportAddr` components), so this
/// composes one from the existing bincode wire encoding.
pub fn encode_endpoint_addr(addr: &EndpointAddr) -> Result<String> {
    let bytes = bincode::serialize(addr).map_err(|e| TransportError::Codec(e.to_string()))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub fn decode_endpoint_addr(s: &str) -> Result<EndpointAddr> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim())
        .map_err(|e| TransportError::Codec(e.to_string()))?;
    bincode::deserialize(&bytes).map_err(|e| TransportError::Codec(e.to_string()))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn endpoint_addr_string_roundtrips() {
        let client = NodeClient::new().await.unwrap();
        let addr = client.addr();

        let encoded = encode_endpoint_addr(&addr).unwrap();
        let decoded = decode_endpoint_addr(&encoded).unwrap();
        assert_eq!(decoded.id, addr.id);
    }

    #[test]
    fn garbage_string_is_rejected_not_panicking() {
        assert!(decode_endpoint_addr("not a valid encoded address").is_err());
    }
}
