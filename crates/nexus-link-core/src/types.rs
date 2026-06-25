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
}

// ---------------------------------------------------------------------------
// Compose command queue (ADR-0049 reverse-agent pattern)
// ---------------------------------------------------------------------------

/// Command types supported by the compose command queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeCommandType {
    GetFile,
    PutFile,
    Activate,
    GetLogsSnapshot,
}

/// A pending command item returned by GET .../compose/commands/pending
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeCommandItem {
    pub id: String,
    #[serde(rename = "type")]
    pub command_type: ComposeCommandType,
    #[serde(default)]
    pub args: serde_json::Value,
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
