use axum::extract::Request;
use axum::{extract::State, http::StatusCode, middleware::Next, response::Response};
use tracing::warn;

use crate::state::SharedState;

/// Bearer token authentication middleware.
/// Validates the incoming nxs_node_* token against the stored token hash in config.
pub async fn require_auth(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            warn!("Missing or invalid Authorization header");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // Validate token format first
    if !nexus_link_core::token::validate_token_format(token) {
        warn!("Invalid token format");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Validate token against the stored node token via constant-time hash comparison
    let stored_hash = nexus_link_core::token::hash_token(&state.config.node.token);
    if !nexus_link_core::token::verify_token(token, &stored_hash) {
        warn!("Token authentication failed");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}
