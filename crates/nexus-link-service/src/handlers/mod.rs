pub mod commands;
pub mod health;

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::SharedState;

/// Build the API route tree
pub fn api_routes() -> Router<SharedState> {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/commands", post(commands::execute_command))
}
