//! Server mailbox binary. Configured entirely via environment variables for
//! this v0 — see `deploy/` (not yet built) for the eventual
//! Docker/systemd-based configuration story.

use pm_node::spawn;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mailbox_key = match std::env::var("PM_NODE_MAILBOX_KEY") {
        Ok(hex_key) => {
            let bytes = hex::decode(hex_key.trim())?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "PM_NODE_MAILBOX_KEY must decode to exactly 32 bytes")?;
            key
        }
        Err(_) => {
            eprintln!(
                "PM_NODE_MAILBOX_KEY not set — this node will not accept any owner-authenticated \
                 requests (Fetch/Ack/RegisterSlot) until it's restarted with one. Generate it \
                 from pm-crypto's Identity::derive(&seed).mailbox_key on the phone this node \
                 belongs to."
            );
            [0u8; 32]
        }
    };

    // Unlike `PM_NODE_MAILBOX_KEY`, this has no insecure-but-working
    // fallback: a random or well-known default would defeat the entire
    // point (a stable, dial-able identity — see `pm_node::spawn`'s docs),
    // and a *shared* well-known key (e.g. all-zero) would let anyone using
    // the same fallback impersonate any unconfigured node at the
    // transport/TLS layer. Required.
    let transport_key_hex = std::env::var("PM_NODE_TRANSPORT_KEY").map_err(|_| {
        "PM_NODE_TRANSPORT_KEY must be set — generate it from pm-crypto's \
         Identity::derive(&seed).server_transport_key on the phone this node belongs \
         to (note: server_transport_key, not transport_key — that one's reserved for \
         the phone's own Local-delivery identity, a deliberately separate key so the \
         phone and this node never claim the same network identity if both run at \
         once). Unlike PM_NODE_MAILBOX_KEY, this has no fallback: it's this node's own \
         network identity, and it must stay the same across restarts for the address \
         you hand out to keep working."
    })?;
    let transport_key_bytes = hex::decode(transport_key_hex.trim())?;
    let transport_key: [u8; 32] = transport_key_bytes
        .try_into()
        .map_err(|_| "PM_NODE_TRANSPORT_KEY must decode to exactly 32 bytes")?;

    let data_dir = match std::env::var("PM_NODE_DATA_DIR") {
        Ok(path) => Some(std::path::PathBuf::from(path)),
        Err(_) => {
            eprintln!(
                "PM_NODE_DATA_DIR not set — this node will run in-memory only, and all mailbox \
                 data will be lost on restart. Set it to a writable directory path to persist \
                 storage across restarts."
            );
            None
        }
    };

    let node = spawn(mailbox_key, transport_key, data_dir.as_deref()).await?;
    node.endpoint().online().await;
    let addr = node.endpoint().addr();
    println!("pm-node listening. Endpoint address (paste this into the app's Server mailbox setup screen):");
    println!("{}", pm_transport::encode_endpoint_addr(&addr)?);

    tokio::signal::ctrl_c().await?;
    println!("shutting down");
    node.shutdown().await?;
    Ok(())
}
