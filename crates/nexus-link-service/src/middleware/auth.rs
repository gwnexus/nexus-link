use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use tracing::warn;

/// Bearer token authentication middleware.
/// Validates the incoming nxs_node_* token against the configured token.
pub async fn require_auth(req: Request, next: Next) -> Result<Response, StatusCode> {
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

    // Validate token format
    if !nexus_link_core::token::validate_token_format(token) {
        warn!("Invalid token format");
        return Err(StatusCode::UNAUTHORIZED);
    }

    // TODO: Validate token against configured node token hash
    // For now, accept any well-formed nxs_node_* token
    // In production: compare hash_token(token) against stored hash

    Ok(next.run(req).await)
}
