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

fn default_tail_lines() -> u32 {
    100
}
