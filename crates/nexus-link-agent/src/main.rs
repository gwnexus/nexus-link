use std::time::Duration;

use nexus_link_core::config::Config;
use nexus_link_core::heartbeat::HEARTBEAT_SEC_RANGE;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod collector;
mod heartbeat;
mod pusher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // --config <path> override (sets NEXUS_LINK_CONFIG before Config::load)
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--config")
        && let Some(path) = args.get(pos + 1)
    {
        unsafe { std::env::set_var("NEXUS_LINK_CONFIG", path) };
    }
    // --profile <name> override (sets NEXUS_LINK_PROFILE before Config::load)
    if let Some(pos) = args.iter().position(|a| a == "--profile")
        && let Some(profile) = args.get(pos + 1)
    {
        unsafe { std::env::set_var("NEXUS_LINK_PROFILE", profile) };
    }

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
    // Validated here, not in Config::load, so `nexus-link config set` can
    // still open and repair a file with an invalid value.
    if !HEARTBEAT_SEC_RANGE.contains(&config.agent.heartbeat_sec) {
        anyhow::bail!(
            "agent.heartbeat_sec = {} is out of range ({}-{} seconds); fix it with \
             `nexus-link config set agent.heartbeat_sec <secs>`",
            config.agent.heartbeat_sec,
            HEARTBEAT_SEC_RANGE.start(),
            HEARTBEAT_SEC_RANGE.end()
        );
    }
    let interval = Duration::from_secs(config.agent.push_sec);
    let heartbeat_interval = Duration::from_secs(config.agent.heartbeat_sec);

    info!(
        node_id = %config.node.node_id,
        api_url = %config.api.base_url,
        push_sec = config.agent.push_sec,
        heartbeat_sec = config.agent.heartbeat_sec,
        models_inventory = config.agent.models_url.is_some(),
        "Agent configured"
    );

    let heartbeat_config = config.clone();

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

    // Online heartbeat loop (carries the served-model inventory, hub >= 0.21.0)
    let heartbeat_handle = tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!("Heartbeat client setup failed: {}", e);
                return;
            }
        };
        let mut ticker = tokio::time::interval(heartbeat_interval);
        loop {
            ticker.tick().await;
            if let Err(e) = heartbeat::send_heartbeat(&client, &heartbeat_config).await {
                error!("Heartbeat failed: {}", e);
            }
        }
    });

    // Wait for shutdown signal
    signal::ctrl_c().await?;
    info!("Shutdown signal received, stopping agent...");
    push_handle.abort();
    heartbeat_handle.abort();

    Ok(())
}
