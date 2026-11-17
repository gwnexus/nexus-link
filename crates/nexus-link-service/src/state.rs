use std::sync::Arc;

use bollard::Docker;
use ed25519_dalek::VerifyingKey;
use nexus_link_core::config::{Config, signing_key_dirs};
use nexus_link_core::signing::{KeyLoad, NonceCache, load_verifying_key};
use nexus_link_core::token::hash_token;
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
        let signing_pubkey = load_signing_pubkey(&config);
        let node_token_hash = hash_token(config.node_token());
        let cmd_token_hash = config.cmd_token().map(hash_token);
        Ok(Self {
            config,
            docker,
            signing_pubkey,
            node_token_hash,
            cmd_token_hash,
            nonces: NonceCache::new(),
        })
    }
}

/// Type alias for shared state
pub type SharedState = Arc<AppState>;

/// Load the Ed25519 verifying key (see `nexus_link_core::signing`):
/// `~/.nexus-link/signing_key.pub` first, then `compose.signing_public_key`.
fn load_signing_pubkey(config: &Config) -> Option<VerifyingKey> {
    let load = load_verifying_key(&config.compose, &signing_key_dirs());
    match &load {
        KeyLoad::Loaded(_) => {}
        KeyLoad::Invalid(source) => warn!(
            source,
            "Ed25519 verifying key could not be parsed (expected base64url, 32 bytes)"
        ),
        KeyLoad::Missing => {}
    }
    if config.compose.require_signatures && load.key().is_none() {
        warn!(
            "compose.require_signatures = true but no Ed25519 public key is configured; \
             all signed command paths will reject commands"
        );
    }
    load.key().copied()
}
