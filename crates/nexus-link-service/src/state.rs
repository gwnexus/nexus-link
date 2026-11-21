use std::sync::Arc;

use bollard::Docker;
use ed25519_dalek::VerifyingKey;
use nexus_link_core::config::{Config, signing_key_dirs};
use nexus_link_core::signing::{KeyLoad, NonceCache, key_fingerprint, load_verifying_keys};
use nexus_link_core::token::hash_token;
use tracing::{info, warn};

/// Shared application state for the axum service
pub struct AppState {
    pub config: Config,
    /// Docker client — available for bollard-based log streaming and container stats.
    #[allow(dead_code)]
    pub docker: Docker,
    /// Ed25519 verifying keys of this profile for signed C&C commands
    /// (ADR-0051 v2, ADR-0008). Empty when none is configured; signature
    /// checks are then skipped or rejected depending on
    /// `config.compose.require_signatures`.
    pub signing_pubkeys: Vec<VerifyingKey>,
    /// Pre-computed SHA-256 hash of the node token (SEC-004).
    pub node_token_hash: String,
    /// Pre-computed SHA-256 hash of the cmd token (SEC-004). None if C&C not configured.
    pub cmd_token_hash: Option<String>,
    /// Nonce replay cache (SEC-005), shared by the HTTP command path and the
    /// command queue so a nonce is accepted once across both.
    pub nonces: NonceCache,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        let signing_pubkeys = load_signing_pubkeys(&config);
        let node_token_hash = hash_token(config.node_token());
        let cmd_token_hash = config.cmd_token().map(hash_token);
        Ok(Self {
            config,
            docker,
            signing_pubkeys,
            node_token_hash,
            cmd_token_hash,
            nonces: NonceCache::new(),
        })
    }
}

/// Type alias for shared state
pub type SharedState = Arc<AppState>;

/// Load the profile's Ed25519 verifying keys (ADR-0008, see
/// `nexus_link_core::signing::load_verifying_keys`): `signing_public_keys`,
/// then `signing_public_key`, then the legacy `signing_key.pub`.
fn load_signing_pubkeys(config: &Config) -> Vec<VerifyingKey> {
    let load = load_verifying_keys(&config.compose, &signing_key_dirs());
    match &load {
        KeyLoad::Loaded(keys) => {
            let fingerprints: Vec<String> = keys.iter().map(key_fingerprint).collect();
            info!(?fingerprints, "Ed25519 verifying keys loaded");
        }
        KeyLoad::Invalid(source) => warn!(
            source,
            "Ed25519 verifying key could not be parsed (expected base64url, 32 bytes)"
        ),
        KeyLoad::Missing => {}
    }
    if config.compose.require_signatures && load.keys().is_empty() {
        warn!(
            "compose.require_signatures = true but no Ed25519 public key is configured; \
             all signed command paths will reject commands"
        );
    }
    load.keys().to_vec()
}
