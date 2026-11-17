use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// System-level config directory (FHS-compliant, root-owned, service reads).
pub const SYSTEM_CONFIG_DIR: &str = "/etc/nexus-link";

/// Runtime state directory (service-owned: keys, PID files, transient state).
pub const SYSTEM_STATE_DIR: &str = "/var/lib/nexus-link";

/// The dedicated system user name for nexus-link services.
pub const SERVICE_USER: &str = "nexus-link";

/// The group name for the nexus-link service user.
pub const SERVICE_GROUP: &str = "nexus-link";

/// Default profile name (backward compatible, resolves to config.toml)
pub const DEFAULT_PROFILE: &str = "default";

/// Env var for profile selection
pub const PROFILE_ENV_VAR: &str = "NEXUS_LINK_PROFILE";

/// Get the active profile name from NEXUS_LINK_PROFILE env var.
/// Returns "default" if not set.
pub fn active_profile() -> String {
    std::env::var(PROFILE_ENV_VAR).unwrap_or_else(|_| DEFAULT_PROFILE.to_string())
}

/// Get the config filename for a given profile.
/// "default" -> "config.toml" (backward compatible)
/// anything else -> "config.<profile>.toml"
pub fn config_filename_for_profile(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "config.toml".to_string()
    } else {
        format!("config.{}.toml", profile)
    }
}

/// Get the state directory for a given profile.
/// Returns /var/lib/nexus-link/<profile>/ (or /var/lib/nexus-link/ for default).
pub fn state_dir_for_profile(profile: &str) -> PathBuf {
    if profile == DEFAULT_PROFILE {
        PathBuf::from(SYSTEM_STATE_DIR)
    } else {
        PathBuf::from(SYSTEM_STATE_DIR).join(profile)
    }
}

/// Get the systemd service name for nexus-link-agent.
/// "default" -> "nexus-link-agent" (backward compatible)
/// anything else -> "nexus-link-agent@<profile>"
pub fn systemd_agent_service_name(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "nexus-link-agent".to_string()
    } else {
        format!("nexus-link-agent@{}", profile)
    }
}

/// Get the systemd service name for nexus-link-service.
pub fn systemd_service_service_name(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "nexus-link-service".to_string()
    } else {
        format!("nexus-link-service@{}", profile)
    }
}

/// Resolve the effective config path. Priority:
/// 1. `NEXUS_LINK_CONFIG` env var (explicit override)
/// 2. System path `/etc/nexus-link/<config_file>` (FHS standard, profile-aware)
/// 3. Legacy system path `/var/lib/nexus-link/config.toml` (default profile only)
/// 4. User path `~/.nexus-link/config.toml` (default profile only)
/// 5. Non-default profile: system path even if it doesn't exist yet
pub fn effective_config_path() -> PathBuf {
    // 1. Explicit env var override (absolute precedence)
    if let Ok(p) = std::env::var("NEXUS_LINK_CONFIG") {
        return PathBuf::from(p);
    }

    let profile = active_profile();
    let config_file = config_filename_for_profile(&profile);

    // 2. System config path (preferred for service mode)
    let system_path = PathBuf::from(SYSTEM_CONFIG_DIR).join(&config_file);
    if system_path.exists() {
        return system_path;
    }

    // 3. Legacy system path (pre-v0.10.2 migration path, default profile only)
    if profile == DEFAULT_PROFILE {
        let legacy_system = PathBuf::from(SYSTEM_STATE_DIR).join("config.toml");
        if legacy_system.exists() {
            return legacy_system;
        }
    }

    // 4. Legacy user path (default profile only)
    if profile == DEFAULT_PROFILE {
        return default_config_path();
    }

    // 5. Non-default profile: return system path even if it doesn't exist yet
    system_path
}

/// System-level config path for the active profile
pub fn system_config_path() -> PathBuf {
    let profile = active_profile();
    PathBuf::from(SYSTEM_CONFIG_DIR).join(config_filename_for_profile(&profile))
}

// ── File permissions (SEC-001) ───────────────────────────────────────────────

/// Mode of the system config dir: root writes, the service group reads.
pub const SYSTEM_CONFIG_DIR_MODE: u32 = 0o750;
/// Mode of files in the system config dir (configs, signing key).
pub const SYSTEM_CONFIG_FILE_MODE: u32 = 0o640;
/// Mode of a user config dir (`~/.nexus-link`).
pub const USER_CONFIG_DIR_MODE: u32 = 0o700;
/// Mode of files in a user config dir.
pub const USER_CONFIG_FILE_MODE: u32 = 0o600;

/// Whether `dir` is the system config dir (`/etc/nexus-link`).
pub fn is_system_config_dir(dir: &std::path::Path) -> bool {
    dir == std::path::Path::new(SYSTEM_CONFIG_DIR)
}

/// Target modes for a file written into `dir`: `(dir_mode, file_mode)`.
pub fn private_file_modes(dir: &std::path::Path) -> (u32, u32) {
    if is_system_config_dir(dir) {
        (SYSTEM_CONFIG_DIR_MODE, SYSTEM_CONFIG_FILE_MODE)
    } else {
        (USER_CONFIG_DIR_MODE, USER_CONFIG_FILE_MODE)
    }
}

/// Write a file that may hold secrets (config, signing key) with the
/// permissions of its directory (see [`private_file_modes`]).
///
/// The file is created `0600` before any content is written, so it is never
/// readable by others in between. In the system config dir, file and dir
/// are then handed to `root:nexus-link`; this needs root and fails with a
/// clear error otherwise. If the `nexus-link` group does not exist yet
/// (setup not run), the file stays `0600` root-only.
pub fn write_private_file(path: &std::path::Path, content: &[u8]) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(USER_CONFIG_FILE_MODE)
            .open(path)
            .map_err(|e| anyhow::anyhow!("Cannot write {}: {}", path.display(), e))?;
        // An existing file keeps its mode on open; tighten before writing.
        file.set_permissions(std::fs::Permissions::from_mode(USER_CONFIG_FILE_MODE))?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);

        let (dir_mode, file_mode) = private_file_modes(dir);
        if is_system_config_dir(dir) {
            match service_group_gid() {
                Some(gid) => {
                    std::os::unix::fs::chown(path, Some(0), Some(gid)).map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot set owner root:{} on {}: {} (run with sudo)",
                            SERVICE_GROUP,
                            path.display(),
                            e
                        )
                    })?;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(file_mode))?;
                    std::os::unix::fs::chown(dir, Some(0), Some(gid)).map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot set owner root:{} on {}: {} (run with sudo)",
                            SERVICE_GROUP,
                            dir.display(),
                            e
                        )
                    })?;
                    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dir_mode))?;
                }
                None => tracing::warn!(
                    path = %path.display(),
                    "group '{}' not found; file stays root-only (run `nexus-link setup`)",
                    SERVICE_GROUP
                ),
            }
        } else {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(file_mode))?;
            // Best effort: the user dir may be shared with other tools.
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dir_mode));
        }
    }

    #[cfg(not(unix))]
    std::fs::write(path, content)?;

    Ok(())
}

/// Numeric gid of the service group, if it exists (`getent group`, which
/// also covers NSS sources; falls back to `/etc/group`).
#[cfg(unix)]
pub fn service_group_gid() -> Option<u32> {
    let from_getent = std::process::Command::new("getent")
        .args(["group", SERVICE_GROUP])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let text = from_getent.or_else(|| std::fs::read_to_string("/etc/group").ok())?;
    parse_group_gid(&text, SERVICE_GROUP)
}

/// Find the gid of `group` in `/etc/group` formatted text.
pub fn parse_group_gid(text: &str, group: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != group {
            return None;
        }
        fields.nth(1)?.trim().parse().ok()
    })
}

/// Directories searched for `signing_key.pub`, most specific first: the dir
/// of the effective config (`/etc/nexus-link` for system installs), the
/// profile's state dir, the system state dir (legacy `setup` location) and
/// `~/.nexus-link`. Duplicates are removed.
pub fn signing_key_dirs() -> Vec<PathBuf> {
    let profile = active_profile();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let candidates = [
        effective_config_path().parent().map(PathBuf::from),
        Some(state_dir_for_profile(&profile)),
        Some(PathBuf::from(SYSTEM_STATE_DIR)),
        Some(dirs_home()),
    ];
    for dir in candidates.into_iter().flatten() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

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
/// heartbeat_sec = 60
/// models_url = "http://127.0.0.1:4000"   # optional, local runtime
///
/// [service]
/// listen_addr = "127.0.0.1"
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

    /// PostgreSQL connection string for LISTEN/NOTIFY wake-up channel (ADR-0071).
    /// Must use session-mode pooling (port 5432, not transaction-mode 6543).
    /// Optional — when absent, the service falls back to HTTP-only polling.
    #[serde(default)]
    pub database_url: Option<String>,

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

    /// Online heartbeat interval in seconds (nexus-link-agent).
    /// Must stay well below the hub's 5-minute offline threshold.
    /// Default: 60
    #[serde(default = "default_heartbeat_sec")]
    pub heartbeat_sec: u64,

    /// Base URL of the local OpenAI-compatible runtime (LiteLLM or vLLM),
    /// e.g. "http://127.0.0.1:4000". Its `/v1/models` list is reported as the
    /// node's model inventory in the heartbeat. Never sent to the backend.
    /// When absent, the heartbeat carries no `models` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_url: Option<String>,

    /// Optional bearer key for the local runtime's `/v1/models` endpoint
    /// (e.g. a LiteLLM key). Used only against `models_url`, never sent to
    /// the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_api_key: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            push_sec: default_push_sec(),
            poll_sec: default_poll_sec(),
            heartbeat_sec: default_heartbeat_sec(),
            models_url: None,
            models_api_key: None,
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
    /// Load config from the effective path (env var > system > user).
    /// Logs the resolved path at info level for debuggability.
    pub fn load() -> anyhow::Result<Self> {
        let path = effective_config_path();
        tracing::info!(config_path = %path.display(), "Config loaded from {}", path.display());
        Self::load_from(path)
    }

    /// Load config from a specific path
    pub fn load_from(path: PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config at {}: {}", path.display(), e))?;
        let mut config: Config = toml::from_str(&content)?;
        config.migrate_legacy_fields();
        Ok(config)
    }

    /// Save config to the effective path
    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(effective_config_path())
    }

    /// Save config to a specific path.
    ///
    /// Permissions (SEC-001, see [`write_private_file`]): in the system
    /// config dir the file becomes `0640 root:nexus-link` and the dir `0750
    /// root:nexus-link`, so the services can read but not modify it; anywhere
    /// else (`~/.nexus-link`) the file is `0600` and the dir `0700`.
    pub fn save_to(&self, path: PathBuf) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        write_private_file(&path, content.as_bytes())
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

fn default_heartbeat_sec() -> u64 {
    60
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
