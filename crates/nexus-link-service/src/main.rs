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
mod pg_listener;
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

    // SEC-006: Warn when signature enforcement is disabled
    if !config.compose.require_signatures && config.compose.cmd_token.is_some() {
        warn!(
            "SECURITY: require_signatures = false — C&C write operations are \
             protected only by the cmd token. Enable Ed25519 signatures for \
             cryptographic proof of origin."
        );
    }

    // SEC-002/SEC-010: Warn when listening on non-localhost without TLS
    if config.service.listen_addr != "127.0.0.1" && config.service.listen_addr != "::1" {
        warn!(
            listen_addr = %config.service.listen_addr,
            "SECURITY: Listening on a non-localhost address without TLS. \
             Bearer tokens will be transmitted in cleartext. Consider binding \
             to 127.0.0.1 or configuring TLS."
        );
    }

    let state = Arc::new(AppState::new(config.clone())?);

    // ── Command queue poll loop + PG LISTEN/NOTIFY wake-up ───────────────
    if config.compose.cmd_token.is_some() {
        let poll_state = Arc::clone(&state);
        let poll_interval = Duration::from_secs(config.agent.poll_sec);

        // Shared wake signal: PG LISTEN notifications trigger immediate poll
        let wake = Arc::new(tokio::sync::Notify::new());

        // Start PG LISTEN/NOTIFY listener (no-op if database_url not configured)
        if config.api.database_url.is_some() {
            let pg_state = Arc::clone(&state);
            let pg_wake = Arc::clone(&wake);
            tokio::spawn(async move {
                pg_listener::run(pg_state, pg_wake).await;
            });
            info!("PG LISTEN/NOTIFY wake-up channel enabled");
        } else {
            info!("PG LISTEN/NOTIFY not configured — HTTP-only polling");
        }

        // Poll loop: runs on interval OR immediately when woken by PG NOTIFY
        let poll_wake = Arc::clone(&wake);
        let default_poll_secs = poll_interval.as_secs();
        tokio::spawn(async move {
            info!(
                interval_s = default_poll_secs,
                "Command queue poll loop started"
            );
            let mut current_interval = poll_interval;
            loop {
                // Wait for either the interval OR a wake signal
                tokio::select! {
                    _ = tokio::time::sleep(current_interval) => {}
                    _ = poll_wake.notified() => {
                        tracing::debug!("Poll triggered by PG NOTIFY wake-up");
                    }
                }
                match poller::poll_and_execute(&poll_state).await {
                    Ok(Some(hint_secs)) if hint_secs > 0 => {
                        let new_interval = Duration::from_secs(hint_secs);
                        if new_interval != current_interval {
                            tracing::debug!(
                                interval_s = hint_secs,
                                "Adjusting poll interval via X-Poll-Interval"
                            );
                            current_interval = new_interval;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Command queue poll error: {}", e);
                        // Reset to default on error
                        current_interval = poll_interval;
                    }
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
