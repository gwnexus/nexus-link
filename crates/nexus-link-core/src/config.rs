use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default config path: ~/.nexus-link/config.toml
pub fn default_config_path() -> PathBuf {
    dirs_home().join("config.toml")
}

/// Nexus-link home directory: ~/.nexus-link/
pub fn dirs_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".nexus-link")
}

/// Main configuration stored in ~/.nexus-link/config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub api: ApiConfig,

    #[serde(default)]
    pub service: ServiceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Node ID assigned during registration
    pub node_id: String,

    /// Human-readable node name
    pub name: String,

    /// Node token (nxs_node_*)
    pub token: String,

    /// Optional tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Nexus backend base URL
    pub base_url: String,

    /// Telemetry push interval in seconds
    #[serde(default = "default_push_interval")]
    pub push_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Listen address for the command receiver
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// Listen port
    #[serde(default = "default_listen_port")]
    pub port: u16,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            port: default_listen_port(),
        }
    }
}

impl Config {
    /// Load config from default path
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(default_config_path())
    }

    /// Load config from a specific path
    pub fn load_from(path: PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config at {}: {}", path.display(), e))?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to default path
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(default_config_path())
    }

    /// Save config to a specific path
    pub fn save_to(&self, path: PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

fn default_push_interval() -> u64 {
    30
}

fn default_listen_addr() -> String {
    "0.0.0.0".to_string()
}

fn default_listen_port() -> u16 {
    8443
}
