pub mod commands;
pub mod compose;
pub mod health;

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::SharedState;

/// Public routes — no authentication required.
pub fn public_routes() -> Router<SharedState> {
    Router::new().route("/health", get(health::health_check))
}

/// Command routes — protected by node token (nxs_node_*).
/// Auth middleware applied in main.rs via from_fn_with_state + require_auth.
pub fn command_routes() -> Router<SharedState> {
    Router::new().route("/commands", post(commands::execute_command))
}

/// Compose routes — protected by C&C token (nxs_cmd_*).
/// Auth middleware applied in main.rs via from_fn_with_state + require_cmd_auth.
pub fn compose_routes() -> Router<SharedState> {
    Router::new()
        .route(
            "/compose/file",
            get(compose::get_compose_file).put(compose::put_compose_file),
        )
        .route("/compose/activate", post(compose::activate_compose))
        .route("/compose/logs", get(compose::stream_compose_logs))
}
