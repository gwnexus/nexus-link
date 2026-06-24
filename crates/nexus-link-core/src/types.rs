use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Command types that can be sent from Nexus backend to the node
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum NodeCommand {
    #[serde(rename = "compose_restart")]
    ComposeRestart(ComposeRestartPayload),

    #[serde(rename = "compose_logs")]
    ComposeLogs(ComposeLogsPayload),

    #[serde(rename = "config_exchange")]
    ConfigExchange(ConfigExchangePayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeRestartPayload {
    /// Optional: specific service to restart (None = all)
    pub service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLogsPayload {
    /// Service name to fetch logs from
    pub service: String,
    /// Number of tail lines
    #[serde(default = "default_tail_lines")]
    pub tail: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigExchangePayload {
    /// Configuration key to exchange
    pub key: String,
    /// Optional new value (None = read-only)
    pub value: Option<serde_json::Value>,
}

/// Response to a command execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Node registration request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub private_ip: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
}

/// Node registration response from backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub node_id: String,
    pub token: String,
}

// ---------------------------------------------------------------------------
// Compose file management types
// ---------------------------------------------------------------------------

/// Metadata entry for a single file in the compose root
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

/// Response for GET /api/compose/files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileListResponse {
    pub compose_root: String,
    pub files: Vec<ComposeFileEntry>,
}

/// Response for GET /api/compose/files/:filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileContent {
    pub filename: String,
    pub content: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

/// Request body for PUT /api/compose/files/:filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileWriteRequest {
    pub content: String,
    /// Optional human-readable commit message (logged server-side)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Response for PUT /api/compose/files/:filename
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileWriteResponse {
    pub success: bool,
    pub filename: String,
    pub size_bytes: u64,
}

/// Response for POST /api/compose/apply
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeApplyResponse {
    pub success: bool,
    pub exit_code: i32,
    pub output: String,
}

/// Response for GET /api/compose/logs[/:service]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLogsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    pub lines: Vec<String>,
}

fn default_tail_lines() -> u32 {
    100
}
