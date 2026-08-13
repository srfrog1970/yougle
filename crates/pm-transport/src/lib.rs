//! `pm-transport`: iroh endpoint setup and the client side of talking to a
//! Server mailbox (`pm-node`) over the wire protocol defined in
//! `pm_proto::node_protocol`. This is a stand-in for what `pm-core` will
//! eventually own — `pm-core` doesn't exist yet (see `docs/PRD.md`'s
//! milestone plan), so M2's integration test uses this crate directly.

pub mod error;

use base64::Engine;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use pm_proto::{DirectAck, NodeRequest, NodeResponse, DIRECT_ALPN, MAX_MESSAGE_SIZE, NODE_ALPN};

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

/// A client endpoint for dialing out to Server mailboxes (and, since M6,
/// directly to a Local contact's own endpoint). One instance is enough for
/// a whole app session; a fresh QUIC connection is opened per call (see
/// `docs/PRD.md`'s Open Items — connection reuse/multiplexing is a later
/// efficiency concern, not a v0 requirement).
pub struct NodeClient {
    endpoint: Endpoint,
}

impl NodeClient {
    /// `secret_key` binds this endpoint's iroh identity deterministically
    /// (rather than the random keypair `Endpoint::bind` alone would
    /// generate) — callers derive it from their seed
    /// (`pm_crypto::Identity::transport_key`) so it's the same every
    /// session. This matters for two reasons: a self-hosted `pm-node`'s
    /// address must stay stable across restarts (it didn't before this —
    /// see `docs/adr` / `app/README.md`), and a Local contact must be
    /// dialable at all, which requires their endpoint identity to be
    /// knowable in advance (shared at pairing time), not random per
    /// launch.
    pub async fn new(secret_key: [u8; 32]) -> Result<Self> {
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(SecretKey::from_bytes(&secret_key))
            .bind()
            .await
            .map_err(|e| TransportError::Endpoint(e.to_string()))?;
        Ok(Self { endpoint })
    }

    /// This endpoint's own address, shareable with a peer so they can dial
    /// back (used by the M2 test to have the "client" also be reachable,
    /// though `NodeClient` itself has no accept-side logic — that's
    /// `pm-node`'s job, or, for direct client-to-client delivery,
    /// `pm-core`'s own `Router` built around a clone of `endpoint()`).
    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// A clone of the underlying endpoint (iroh's `Endpoint` clones
    /// cheaply — it's `Arc`-backed internally), so a caller can register
    /// its own accept-side `Router` on the *same* endpoint this
    /// `NodeClient` dials out from, without `NodeClient` needing to know
    /// anything about accept-side protocols itself.
    pub fn endpoint(&self) -> Endpoint {
        self.endpoint.clone()
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

    /// Delivers one padded envelope directly to a peer's own endpoint over
    /// [`DIRECT_ALPN`] — the Local-to-local path (`docs/PRD.md` Flow 3),
    /// as opposed to `call`'s mailbox request/response. `Ok(())` only on
    /// an explicit `DirectAck::Ok` from the peer; anything else (a stream
    /// error, `DirectAck::Error`, or the caller's own timeout wrapping
    /// this) leaves the message undelivered, which `pm-core` treats as a
    /// send failure — nothing is retried here.
    pub async fn call_direct(
        &self,
        peer_addr: EndpointAddr,
        envelope_bytes: Vec<u8>,
    ) -> Result<()> {
        let conn = self
            .endpoint
            .connect(peer_addr, DIRECT_ALPN)
            .await
            .map_err(|e| TransportError::Connect(e.to_string()))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;

        send.write_all(&envelope_bytes)
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;
        send.finish()
            .map_err(|e| TransportError::Stream(e.to_string()))?;

        let ack_bytes = recv
            .read_to_end(MAX_MESSAGE_SIZE)
            .await
            .map_err(|e| TransportError::Stream(e.to_string()))?;
        let ack: DirectAck =
            bincode::deserialize(&ack_bytes).map_err(|e| TransportError::Codec(e.to_string()))?;

        conn.close(0u32.into(), b"done");

        match ack {
            DirectAck::Ok => Ok(()),
            DirectAck::Error(e) => Err(TransportError::Rejected(e)),
        }
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
        let client = NodeClient::new([1u8; 32]).await.unwrap();
        let addr = client.addr();

        let encoded = encode_endpoint_addr(&addr).unwrap();
        let decoded = decode_endpoint_addr(&encoded).unwrap();
        assert_eq!(decoded.id, addr.id);
    }

    #[tokio::test]
    async fn same_secret_key_produces_the_same_endpoint_id() {
        let a = NodeClient::new([7u8; 32]).await.unwrap();
        let b = NodeClient::new([7u8; 32]).await.unwrap();
        assert_eq!(a.addr().id, b.addr().id);
    }

    #[test]
    fn garbage_string_is_rejected_not_panicking() {
        assert!(decode_endpoint_addr("not a valid encoded address").is_err());
    }
}
