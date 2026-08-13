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

    let (router, _store) = spawn(mailbox_key).await?;
    router.endpoint().online().await;
    let addr = router.endpoint().addr();
    println!("pm-node listening. Endpoint address (paste this into the app's Server mailbox setup screen):");
    println!("{}", pm_transport::encode_endpoint_addr(&addr)?);

    tokio::signal::ctrl_c().await?;
    println!("shutting down");
    router.shutdown().await?;
    Ok(())
}
