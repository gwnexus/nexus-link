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
///
/// On-disk format (v0.8.4+):
/// ```toml
/// [node]
/// node_id = "..."
/// name    = "spark-ccd9"
///
/// [api]
/// base_url = "https://nexus.gatewarden.eu"
///
/// [api.tokens]
/// telemetry = { token = "nxs_node_...", scope = "read" }
/// command   = { token = "nxs_cmd_...",  scope = "read_write" }
///
/// [agent]
/// push_sec = 6
/// poll_sec = 2
///
/// [service]
/// listen_addr = "0.0.0.0"
/// port        = 8443
///
/// [compose]
/// dir                = "/opt/dgx-llm"
/// extra_extensions   = [".env", ".conf", ".toml"]
/// signing_public_key = "..."
/// require_signatures = false
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node: NodeConfig,
    pub api: ApiConfig,

    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub service: ServiceConfig,

    #[serde(default)]
    pub compose: ComposeConfig,
}

// ---------------------------------------------------------------------------
// [api] + [api.tokens]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Nexus backend base URL
    pub base_url: String,

    /// Token credentials for the two nexus-link channels.
    #[serde(default)]
    pub tokens: ApiTokens,

    // ── Backward-compatibility aliases ────────────────────────────────────
    // Old config.toml files write push_interval_secs directly under [api].
    // We deserialize them here and migrate on first save.
    /// @deprecated — use [agent] push_sec instead. Kept for migration only.
    #[serde(default, skip_serializing)]
    pub push_interval_secs: Option<u64>,
}

/// [api.tokens] — inline tables for the two credential channels.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiTokens {
    /// Telemetry channel (nxs_node_*) — read-only, push-based.
    #[serde(default)]
    pub telemetry: Option<TokenEntry>,

    /// Command channel (nxs_cmd_*) — read_write, poll-based.
    #[serde(default)]
    pub command: Option<TokenEntry>,
}

/// A single token entry in [api.tokens].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEntry {
    pub token: String,
    /// "read" or "read_write"
    pub scope: String,
}

// ---------------------------------------------------------------------------
// [agent] — runtime behavior (intervals)
// ---------------------------------------------------------------------------

/// [agent] — controls push and poll intervals for both daemons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Telemetry push interval in seconds (nexus-link-agent).
    /// Default: 6
    #[serde(default = "default_push_sec")]
    pub push_sec: u64,

    /// Command queue poll interval in seconds (nexus-link-service).
    /// Default: 2
    #[serde(default = "default_poll_sec")]
    pub poll_sec: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            push_sec: default_push_sec(),
            poll_sec: default_poll_sec(),
        }
    }
}

// ---------------------------------------------------------------------------
// [node]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Node ID assigned during registration
    pub node_id: String,

    /// Human-readable node name
    pub name: String,

    /// Node token (nxs_node_*) — kept for backward compatibility.
    /// New configs use [api.tokens.telemetry].token instead.
    /// On save, this field is kept in sync with api.tokens.telemetry.token.
    pub token: String,

    /// Optional tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// [compose]
// ---------------------------------------------------------------------------

/// Configuration for Docker Compose management and C&C channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// Directory containing docker-compose.yaml and related config files.
    /// Default: /opt/dgx-llm
    #[serde(default = "default_compose_dir")]
    pub dir: PathBuf,

    /// Extra file extensions to expose alongside the compose file.
    #[serde(default = "default_compose_extra_extensions")]
    pub extra_extensions: Vec<String>,

    /// C&C channel token (nxs_cmd_*).
    /// Kept for backward compatibility — new configs use [api.tokens.command].token.
    #[serde(default)]
    pub cmd_token: Option<String>,

    /// Ed25519 public key (base64url, 32 bytes) for signed command verification.
    #[serde(default)]
    pub signing_public_key: Option<String>,

    /// Enforce Ed25519 signatures on write operations (default: false).
    #[serde(default)]
    pub require_signatures: bool,
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            dir: default_compose_dir(),
            extra_extensions: default_compose_extra_extensions(),
            cmd_token: None,
            signing_public_key: None,
            require_signatures: false,
        }
    }
}

// ---------------------------------------------------------------------------
// [service]
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Config impl
// ---------------------------------------------------------------------------

impl Config {
    /// Load config from default path
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(default_config_path())
    }

    /// Load config from a specific path
    pub fn load_from(path: PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config at {}: {}", path.display(), e))?;
        let mut config: Config = toml::from_str(&content)?;
        config.migrate_legacy_fields();
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

    /// Migrate fields from old config layout to new layout.
    ///
    /// Old layout:                         New layout:
    ///   [api]                               [api.tokens]
    ///   push_interval_secs = 10             telemetry = { token = "...", scope = "read" }
    ///   [node]                              command   = { token = "...", scope = "read_write" }
    ///   token = "nxs_node_..."              [agent]
    ///   [compose]                           push_sec = 6
    ///   cmd_token = "nxs_cmd_..."           poll_sec = 2
    fn migrate_legacy_fields(&mut self) {
        // api.push_interval_secs → agent.push_sec
        if let Some(old_interval) = self.api.push_interval_secs.take()
            && self.agent.push_sec == default_push_sec()
        {
            self.agent.push_sec = old_interval;
        }

        // node.token → api.tokens.telemetry.token (if not already set)
        if self.api.tokens.telemetry.is_none() && !self.node.token.is_empty() {
            self.api.tokens.telemetry = Some(TokenEntry {
                token: self.node.token.clone(),
                scope: "read".to_string(),
            });
        }

        // compose.cmd_token → api.tokens.command.token (if not already set)
        if self.api.tokens.command.is_none()
            && let Some(ref cmd) = self.compose.cmd_token.clone()
        {
            self.api.tokens.command = Some(TokenEntry {
                token: cmd.clone(),
                scope: "read_write".to_string(),
            });
        }
    }

    /// Convenience: return the effective node token
    /// (new location: api.tokens.telemetry.token, fallback: node.token)
    pub fn node_token(&self) -> &str {
        self.api
            .tokens
            .telemetry
            .as_ref()
            .map(|t| t.token.as_str())
            .unwrap_or(&self.node.token)
    }

    /// Convenience: return the effective cmd token
    /// (new location: api.tokens.command.token, fallback: compose.cmd_token)
    pub fn cmd_token(&self) -> Option<&str> {
        self.api
            .tokens
            .command
            .as_ref()
            .map(|t| t.token.as_str())
            .or(self.compose.cmd_token.as_deref())
    }
}

// ---------------------------------------------------------------------------
// Default value functions
// ---------------------------------------------------------------------------

fn default_push_sec() -> u64 {
    6
}

fn default_poll_sec() -> u64 {
    2
}

fn default_listen_addr() -> String {
    "127.0.0.1".to_string()
}

fn default_listen_port() -> u16 {
    8443
}

fn default_compose_dir() -> PathBuf {
    PathBuf::from("/opt/dgx-llm")
}

fn default_compose_extra_extensions() -> Vec<String> {
    vec![".env".into(), ".conf".into(), ".toml".into()]
}
