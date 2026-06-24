use std::sync::Arc;

use bollard::Docker;
use nexus_link_core::config::Config;

/// Shared application state for the axum service
pub struct AppState {
    pub config: Config,
    pub docker: Docker,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self { config, docker })
    }
}

/// Type alias for shared state
pub type SharedState = Arc<AppState>;
