use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{Router, extract::DefaultBodyLimit, middleware as axum_mw};
use nexus_link_core::config::Config;
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod handlers;
mod middleware;
mod poller;
mod state;

use middleware::auth::require_auth;
use middleware::cmd_auth::require_cmd_auth;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("nexus_link_service=info".parse()?)
                .add_directive("tower_http=debug".parse()?),
        )
        .json()
        .init();

    info!("nexus-link-service starting...");

    let config = Config::load()?;
    let addr = SocketAddr::new(config.service.listen_addr.parse()?, config.service.port);

    info!(
        compose_root   = %config.compose.dir.display(),
        cmd_channel    = config.compose.cmd_token.is_some(),
        signatures     = config.compose.require_signatures,
        poll_sec       = config.agent.poll_sec,
        "Compose channel configured"
    );

    let state = Arc::new(AppState::new(config.clone())?);

    // ── Command queue poll loop ────────────────────────────────────────────
    if config.compose.cmd_token.is_some() {
        let poll_state = Arc::clone(&state);
        let poll_interval = Duration::from_secs(config.agent.poll_sec);
        tokio::spawn(async move {
            info!(
                interval_s = poll_interval.as_secs(),
                "Command queue poll loop started"
            );
            let mut interval = tokio::time::interval(poll_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = poller::poll_and_execute(&poll_state).await {
                    warn!("Command queue poll error: {}", e);
                }
            }
        });
    } else {
        info!("C&C channel not configured — command queue poll loop disabled");
    }

    // ── HTTP server ────────────────────────────────────────────────────────
    let command_routes = handlers::command_routes().layer(axum_mw::from_fn_with_state(
        Arc::clone(&state),
        require_auth,
    ));

    let compose_routes = handlers::compose_routes().layer(axum_mw::from_fn_with_state(
        Arc::clone(&state),
        require_cmd_auth,
    ));

    let app = Router::new()
        .nest(
            "/api",
            handlers::public_routes()
                .merge(command_routes)
                .merge(compose_routes),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // 2 MiB
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    info!(%addr, "Listening for commands");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Service stopped");
    Ok(())
}

async fn shutdown_signal() {
    signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    info!("Shutdown signal received");
}
