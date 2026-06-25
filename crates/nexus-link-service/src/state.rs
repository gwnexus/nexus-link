use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bollard::Docker;
use ed25519_dalek::VerifyingKey;
use nexus_link_core::config::{Config, dirs_home};
use tracing::warn;

/// Shared application state for the axum service
pub struct AppState {
    pub config: Config,
    /// Docker client — available for bollard-based log streaming and container stats.
    #[allow(dead_code)]
    pub docker: Docker,
    /// Ed25519 verifying key for signed C&C commands (ADR-0051 v2).
    /// None when no signing_key.pub is present — signature checks are
    /// skipped or rejected depending on `config.compose.require_signatures`.
    pub signing_pubkey: Option<VerifyingKey>,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        let signing_pubkey = load_signing_pubkey(&config);
        Ok(Self { config, docker, signing_pubkey })
    }
}

/// Type alias for shared state
pub type SharedState = Arc<AppState>;

/// Load the Ed25519 verifying key.
///
/// Priority:
///   1. `~/.nexus-link/signing_key.pub`  (base64url-encoded 32-byte key)
///   2. `config.compose.signing_public_key` (same format, inline in config.toml)
///
/// Returns `None` and logs a warning if neither source is available.
fn load_signing_pubkey(config: &Config) -> Option<VerifyingKey> {
    // Priority 1: file on disk
    let key_path = dirs_home().join("signing_key.pub");
    if let Ok(content) = std::fs::read_to_string(&key_path) {
        if let Some(key) = decode_verifying_key(content.trim()) {
            return Some(key);
        }
        warn!(
            path = %key_path.display(),
            "signing_key.pub exists but could not be parsed as a base64url Ed25519 verifying key"
        );
    }

    // Priority 2: config field
    if let Some(ref b64) = config.compose.signing_public_key {
        if let Some(key) = decode_verifying_key(b64.trim()) {
            return Some(key);
        }
        warn!(
            "compose.signing_public_key in config.toml could not be parsed as \
             a base64url Ed25519 verifying key"
        );
    }

    // Neither source available
    if config.compose.require_signatures {
        warn!(
            "compose.require_signatures = true but no Ed25519 public key is configured; \
             write operations will be rejected with 403"
        );
    }

    None
}

/// Decode a base64url string into an Ed25519 `VerifyingKey`.
fn decode_verifying_key(b64: &str) -> Option<VerifyingKey> {
    let bytes = URL_SAFE_NO_PAD.decode(b64).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}
