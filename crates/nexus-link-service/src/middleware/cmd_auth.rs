use axum::{
    body::Bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::Verifier;
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::state::SharedState;

/// C&C channel authentication + optional Ed25519 signature middleware.
///
/// Gate 1 (all requests): nxs_cmd_* Bearer token validation (ADR-0051 v1)
/// Gate 2 (write requests only): Ed25519 signature verification (ADR-0051 v2)
///
/// Behavior matrix:
/// ┌───────────────────────┬───────────────┬────────────────┬──────────────────────┐
/// │ require_signatures    │ signing_pubkey│ write op       │ result               │
/// ├───────────────────────┼───────────────┼────────────────┼──────────────────────┤
/// │ false                 │ any           │ any            │ skip sig check       │
/// │ true                  │ None          │ any            │ 403 — key not loaded │
/// │ true                  │ Some          │ missing headers│ 403 — sig required   │
/// │ true                  │ Some          │ bad timestamp  │ 403 — replay         │
/// │ true                  │ Some          │ valid sig      │ 200 proceed          │
/// │ any                   │ any           │ GET            │ skip sig check       │
/// └───────────────────────┴───────────────┴────────────────┴──────────────────────┘
pub async fn require_cmd_auth(
    State(state): State<SharedState>,
    req: Request,
    next: Next,
) -> Result<Response, Response> {
    // -----------------------------------------------------------------------
    // Gate 1a: C&C channel must be configured.
    //   Return 403 — the channel is simply not activated on this node yet.
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
    // Gate 1b: Extract and validate the Bearer token.
    // -----------------------------------------------------------------------
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            warn!("C&C auth: missing or malformed Authorization header");
            return Err(StatusCode::UNAUTHORIZED.into_response());
        }
    };

    if !nexus_link_core::token::validate_cmd_token_format(token) {
        warn!("C&C auth: token does not carry nxs_cmd_* prefix");
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    let expected_hash = nexus_link_core::token::hash_token(stored_cmd_token);
    if !nexus_link_core::token::verify_token(token, &expected_hash) {
        warn!("C&C auth: token verification failed");
        return Err(StatusCode::UNAUTHORIZED.into_response());
    }

    // -----------------------------------------------------------------------
    // Gate 2: Ed25519 signature verification for write operations.
    //   Only active when require_signatures = true AND signing_pubkey is loaded.
    // -----------------------------------------------------------------------
    let method = req.method().clone();
    let is_write = method == Method::PUT || method == Method::POST;

    if is_write && state.config.compose.require_signatures {
        let pubkey = match &state.signing_pubkey {
            Some(k) => k,
            None => {
                warn!("C&C auth: require_signatures=true but no Ed25519 key loaded");
                return Err((
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({
                        "error": "Signature enforcement enabled but no Ed25519 key is configured",
                        "hint": "Re-register the node or set compose.signing_public_key in config"
                    })),
                )
                    .into_response());
            }
        };

        // Extract signature headers
        let sig_hex = req
            .headers()
            .get("x-nexus-signature")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let timestamp_str = req
            .headers()
            .get("x-nexus-timestamp")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let nonce = req
            .headers()
            .get("x-nexus-nonce")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        match (sig_hex, timestamp_str, nonce) {
            (Some(sig_hex), Some(ts_str), Some(nonce_str)) => {
                // Replay protection: ±5 minute window
                match ts_str.parse::<DateTime<Utc>>() {
                    Ok(ts) => {
                        let delta = (Utc::now() - ts).abs();
                        if delta > Duration::minutes(5) {
                            warn!(
                                timestamp = %ts_str,
                                delta_secs = delta.num_seconds(),
                                "C&C auth: request timestamp outside ±5 minute window"
                            );
                            return Err((
                                StatusCode::FORBIDDEN,
                                axum::Json(json!({
                                    "error": "Request timestamp outside ±5 minute replay window"
                                })),
                            )
                                .into_response());
                        }
                    }
                    Err(_) => {
                        warn!("C&C auth: could not parse X-Nexus-Timestamp");
                        return Err((
                            StatusCode::FORBIDDEN,
                            axum::Json(json!({ "error": "Invalid X-Nexus-Timestamp format" })),
                        )
                            .into_response());
                    }
                }

                // Buffer the body to compute its hash, then put it back
                let path = req.uri().path().to_owned();
                let (parts, body) = req.into_parts();

                let body_bytes: Bytes = match axum::body::to_bytes(body, 2 * 1024 * 1024).await {
                    Ok(b) => b,
                    Err(_) => {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            axum::Json(json!({ "error": "Failed to buffer request body" })),
                        )
                            .into_response());
                    }
                };

                // Canonical message: METHOD\nPATH\nTIMESTAMP\nNONCE\nSHA256_HEX(body)
                let body_hash = hex::encode(Sha256::digest(&body_bytes));
                let canonical = format!(
                    "{}\n{}\n{}\n{}\n{}",
                    method.as_str(),
                    path,
                    ts_str,
                    nonce_str,
                    body_hash
                );

                // Decode and verify the Ed25519 signature
                let sig_bytes = match hex::decode(&sig_hex) {
                    Ok(b) if b.len() == 64 => {
                        let mut arr = [0u8; 64];
                        arr.copy_from_slice(&b);
                        arr
                    }
                    _ => {
                        warn!("C&C auth: X-Nexus-Signature is not valid hex or wrong length");
                        return Err((
                            StatusCode::FORBIDDEN,
                            axum::Json(json!({ "error": "Invalid X-Nexus-Signature" })),
                        )
                            .into_response());
                    }
                };

                let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
                if pubkey.verify(canonical.as_bytes(), &signature).is_err() {
                    warn!("C&C auth: Ed25519 signature verification failed");
                    return Err((
                        StatusCode::FORBIDDEN,
                        axum::Json(json!({ "error": "Invalid request signature" })),
                    )
                        .into_response());
                }

                // Reassemble the request with the buffered body
                let req = Request::from_parts(parts, axum::body::Body::from(body_bytes));
                return Ok(next.run(req).await);
            }
            _ => {
                warn!("C&C auth: write operation missing signature headers");
                return Err((
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({
                        "error": "Signature required for write operations",
                        "required_headers": ["X-Nexus-Signature", "X-Nexus-Timestamp", "X-Nexus-Nonce"]
                    })),
                )
                    .into_response());
            }
        }
    }

    Ok(next.run(req).await)
}
