use std::time::Duration;

use nexus_link_core::config::Config;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod collector;
mod pusher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("nexus_link_agent=info".parse()?)
                .add_directive("nexus_link_core=info".parse()?),
        )
        .json()
        .init();

    info!("nexus-link-agent starting...");

    let config = Config::load()?;
    let interval = Duration::from_secs(config.agent.push_sec);

    info!(
        node_id = %config.node.node_id,
        api_url = %config.api.base_url,
        push_sec = config.agent.push_sec,
        "Agent configured"
    );

    // Telemetry push loop
    let push_handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;

            match collector::collect_telemetry(&config).await {
                Ok(payload) => {
                    if let Err(e) = pusher::push_telemetry(&config, &payload).await {
                        error!("Telemetry push failed: {}", e);
                    }
                }
                Err(e) => {
                    error!("Telemetry collection failed: {}", e);
                }
            }
        }
    });

    // Wait for shutdown signal
    signal::ctrl_c().await?;
    info!("Shutdown signal received, stopping agent...");
    push_handle.abort();

    Ok(())
}
