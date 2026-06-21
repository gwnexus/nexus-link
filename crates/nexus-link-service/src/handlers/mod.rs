pub mod commands;
pub mod health;

use axum::{
    Router, middleware as axum_mw,
    routing::{get, post},
};

use crate::middleware::auth::require_auth;
use crate::state::SharedState;

/// Build the API route tree
pub fn api_routes() -> Router<SharedState> {
    let protected = Router::new()
        .route("/commands", post(commands::execute_command))
        .layer(axum_mw::from_fn(require_auth));

    Router::new()
        .route("/health", get(health::health_check))
        .merge(protected)
}
