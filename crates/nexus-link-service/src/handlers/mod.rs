pub mod commands;
pub mod compose;
pub mod health;

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::SharedState;

/// Build the public (unauthenticated) API routes.
pub fn public_routes() -> Router<SharedState> {
    Router::new().route("/health", get(health::health_check))
}

/// Build the protected API routes (auth middleware applied in main.rs).
pub fn protected_routes() -> Router<SharedState> {
    Router::new()
        // Legacy command endpoint
        .route("/commands", post(commands::execute_command))
        // Compose file management
        .route(
            "/compose/file",
            get(compose::get_compose_file).put(compose::put_compose_file),
        )
        // Compose activate (docker compose up -d)
        .route("/compose/activate", post(compose::activate_compose))
        // Compose log stream (SSE)
        .route("/compose/logs", get(compose::stream_compose_logs))
}
