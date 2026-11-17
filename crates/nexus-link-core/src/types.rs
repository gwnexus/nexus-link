use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Legacy command types (POST /api/commands — inbound from Nexus backend)
// ---------------------------------------------------------------------------

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
    pub service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLogsPayload {
    pub service: String,
    #[serde(default = "default_tail_lines")]
    pub tail: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigExchangePayload {
    pub key: String,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub private_ip: Option<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub node_id: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_public_key: Option<String>,
    /// PG connection string for LISTEN/NOTIFY wake-up channel (ADR-0071).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_url: Option<String>,
}

// ---------------------------------------------------------------------------
// Compose command queue (ADR-0049 reverse-agent pattern)
// ---------------------------------------------------------------------------

/// Command types supported by the compose command queue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeCommandType {
    #[default]
    GetFile,
    PutFile,
    Activate,
    GetLogsSnapshot,
    ValidateYaml,
}

impl ComposeCommandType {
    /// Wire name, as used in the JSON `type` field and the signed message.
    pub fn as_str(&self) -> &'static str {
        match self {
            ComposeCommandType::GetFile => "get_file",
            ComposeCommandType::PutFile => "put_file",
            ComposeCommandType::Activate => "activate",
            ComposeCommandType::GetLogsSnapshot => "get_logs_snapshot",
            ComposeCommandType::ValidateYaml => "validate_yaml",
        }
    }
}

/// A pending command item returned by GET .../compose/commands/pending
///
/// Signed items (see `crate::signing`) carry the arguments as the exact JSON
/// string the hub signed in `payload`, plus `signature`, `timestamp` and
/// `nonce`. Unsigned items from older hubs only carry `args`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeCommandItem {
    pub id: String,
    #[serde(rename = "type")]
    pub command_type: ComposeCommandType,
    #[serde(default)]
    pub args: serde_json::Value,
    /// Arguments as the signed JSON string; takes precedence over `args`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Hex-encoded 64-byte Ed25519 signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// RFC 3339 signing time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Unique per signature (replay protection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// Result the device sends to PATCH .../compose/commands/:id
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeCommandResult {
    pub status: ComposeCommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeCommandStatus {
    Completed,
    Failed,
}

// ---------------------------------------------------------------------------
// Legacy compose file management types (kept for local API compat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileEntry {
    pub filename: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileListResponse {
    pub compose_root: String,
    pub files: Vec<ComposeFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileContent {
    pub filename: String,
    pub content: String,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileWriteRequest {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeFileWriteResponse {
    pub success: bool,
    pub filename: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeApplyResponse {
    pub success: bool,
    pub exit_code: i32,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLogsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    pub lines: Vec<String>,
}

fn default_tail_lines() -> u32 {
    100
}
