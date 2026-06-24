use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bollard::Docker;
use nexus_link_core::config::{Config, dirs_home};
use tracing::warn;

/// Shared application state for the axum service
pub struct AppState {
    pub config: Config,
    /// Docker client — available for bollard-based log streaming and container stats.
    #[allow(dead_code)]
    pub docker: Docker,
    /// Ed25519 verifying key for signed C&C commands (ADR-0051 v2).
    /// None when no signing_key.pub is present — signature checks are skipped.
    /// Will be read by compose write-route handlers once v2 is implemented.
    #[allow(dead_code)]
    pub signing_pubkey: Option<SigningPubkey>,
}

/// Raw bytes of an Ed25519 verifying key (32 bytes).
/// Stored as plain bytes so the service compiles without ed25519-dalek in v1.
/// Consumed by the signature verification middleware in v2.
#[allow(dead_code)]
pub struct SigningPubkey(pub [u8; 32]);

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        let signing_pubkey = load_signing_pubkey(&config);
        Ok(Self { config, docker, signing_pubkey })
    }
}

/// Type alias for shared state
pub type SharedState = Arc<AppState>;

/// Load the Ed25519 verifying key from disk (~/.nexus-link/signing_key.pub)
/// with a fallback to config.compose.signing_public_key.
/// Returns None and logs a warning if neither source is available.
fn load_signing_pubkey(config: &Config) -> Option<SigningPubkey> {
    // Priority 1: file
    let key_path = dirs_home().join("signing_key.pub");
    if let Ok(content) = std::fs::read_to_string(&key_path) {
        let trimmed = content.trim();
        if let Ok(bytes) = URL_SAFE_NO_PAD.decode(trimmed) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Some(SigningPubkey(arr));
            }
        }
        warn!(
            path = %key_path.display(),
            "signing_key.pub exists but could not be parsed as base64url Ed25519 key"
        );
    }

    // Priority 2: config field
    if let Some(ref b64) = config.compose.signing_public_key {
        if let Ok(bytes) = URL_SAFE_NO_PAD.decode(b64.trim()) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Some(SigningPubkey(arr));
            }
        }
        warn!("compose.signing_public_key in config.toml could not be parsed as base64url Ed25519 key");
    }

    // Neither source available — signature verification will be disabled
    if config.compose.require_signatures {
        warn!(
            "compose.require_signatures = true but no Ed25519 public key is configured; \
             write operations will be rejected with 403"
        );
    }

    None
}
