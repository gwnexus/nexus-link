use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;
use tracing::warn;

use crate::state::SharedState;

/// C&C channel authentication middleware for /api/compose/* routes.
///
/// Enforces the `nxs_cmd_*` token contract defined in ADR-0051:
/// - 403 Forbidden if cmd_token is not configured (C&C channel not activated)
/// - 401 Unauthorized if the Bearer token is missing or malformed
/// - 401 Unauthorized if the token does not match the stored cmd_token hash
///
/// Note: Ed25519 signature verification (ADR-0051 v2) for write operations
/// is handled separately in the compose handlers when `require_signatures`
/// is enabled in config.
pub async fn require_cmd_auth(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    // -----------------------------------------------------------------------
    // 1. Check that the C&C channel is configured at all.
    //    Return 403 (not 401) — identity is not the issue, the channel is
    //    simply not activated on this node yet.
    // -----------------------------------------------------------------------
    let Some(ref stored_cmd_token) = state.config.compose.cmd_token else {
        warn!("C&C channel not configured: compose.cmd_token is absent");
        return Err((
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "error": "C&C channel not configured on this node",
                "hint": "Run: nexus-link config set compose.cmd_token <nxs_cmd_*>"
            })),
        )
            .into_response());
    };

    // -----------------------------------------------------------------------
    // 2. Extract the Bearer token from the Authorization header.
    // -----------------------------------------------------------------------
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            warn!("C&C auth: missing or malformed Authorization header");
            return Err((StatusCode::UNAUTHORIZED).into_response());
        }
    };

    // -----------------------------------------------------------------------
    // 3. Validate that the token carries the nxs_cmd_* prefix.
    // -----------------------------------------------------------------------
    if !nexus_link_core::token::validate_cmd_token_format(token) {
        warn!("C&C auth: token does not carry nxs_cmd_* prefix");
        return Err((StatusCode::UNAUTHORIZED).into_response());
    }

    // -----------------------------------------------------------------------
    // 4. Verify the token hash against the stored cmd_token.
    //    We hash the stored token on every request (cheap — SHA-256 of ~52
    //    bytes) rather than storing a pre-computed hash in memory, keeping
    //    the config structure simple.
    // -----------------------------------------------------------------------
    let expected_hash = nexus_link_core::token::hash_token(stored_cmd_token);
    if !nexus_link_core::token::verify_token(token, &expected_hash) {
        warn!("C&C auth: token verification failed");
        return Err((StatusCode::UNAUTHORIZED).into_response());
    }

    Ok(next.run(req).await)
}
