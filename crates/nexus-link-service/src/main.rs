use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, middleware as axum_mw};
use nexus_link_core::config::Config;
use tokio::signal;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod handlers;
mod middleware;
mod state;

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
        compose_root = %config.compose.dir.display(),
        "Compose root configured"
    );

    let state = Arc::new(AppState::new(config)?);

    // Protected routes have the auth middleware applied with access to SharedState.
    // from_fn_with_state is called here (after state is constructed) so the closure
    // can extract State<SharedState> via axum's standard extractor mechanism.
    let protected = handlers::protected_routes()
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::auth::require_auth,
        ));

    let app = Router::new()
        .nest("/api", handlers::public_routes().merge(protected))
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
