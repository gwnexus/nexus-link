//! Node heartbeat payload (`POST /api/nodes/{id}/heartbeat`).
//!
//! Since Nexus Hub 0.21.0 the heartbeat may carry a `models` field: the
//! inventory of models the node's local OpenAI-compatible runtime (LiteLLM
//! or vLLM) serves. The hub only uses it for "Add to gateway" suggestions.
//!
//! Rules (agreed with the hub):
//! - Each entry is a plain id string or `{ id, max_model_len }`.
//! - Ids use letters, digits and `. _ : / @ + -`, at most 200 characters.
//! - At most 50 entries; invalid and duplicate ids are dropped.
//! - `models` is omitted (not sent as an empty list) when the runtime could
//!   not be queried, so a temporary failure keeps the last known inventory.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Maximum number of model entries the hub keeps per node.
pub const MAX_MODELS: usize = 50;

/// Maximum length of a model id accepted by the hub.
pub const MAX_MODEL_ID_LEN: usize = 200;

/// Allowed heartbeat interval in seconds. The hub marks a node offline after
/// five minutes without a heartbeat, so the upper bound leaves room for
/// missed beats.
pub const HEARTBEAT_SEC_RANGE: std::ops::RangeInclusive<u64> = 1..=120;

/// One served model as reported in the heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ModelEntry {
    /// Model id only (the runtime does not expose a context window).
    Id(String),
    /// Model id with its context window in tokens.
    Detailed { id: String, max_model_len: u64 },
}

impl ModelEntry {
    pub fn id(&self) -> &str {
        match self {
            ModelEntry::Id(id) | ModelEntry::Detailed { id, .. } => id,
        }
    }
}

/// Request body of the heartbeat endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatPayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelEntry>>,
}

impl HeartbeatPayload {
    /// An `online` heartbeat; pass `None` when the model inventory is unknown.
    pub fn online(models: Option<Vec<ModelEntry>>) -> Self {
        Self {
            status: "online".to_string(),
            models,
        }
    }
}

/// Relevant part of the heartbeat response.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HeartbeatResponse {
    /// Number of stored model entries; present when `models` was sent.
    #[serde(default)]
    pub models_recorded: Option<u64>,
}

/// Whether `id` is a model id the hub accepts.
pub fn is_valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_MODEL_ID_LEN
        && id.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-')
        })
}

/// Model list URL for an OpenAI-compatible runtime base URL.
///
/// Accepts the base with or without a trailing `/v1`
/// (`http://127.0.0.1:4000` and `http://127.0.0.1:4000/v1` both resolve to
/// `http://127.0.0.1:4000/v1/models`).
pub fn models_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelObject>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
    #[serde(default)]
    max_model_len: Option<u64>,
}

/// Parse an OpenAI-compatible `GET /v1/models` response into heartbeat entries.
///
/// Drops invalid and duplicate ids and keeps at most [`MAX_MODELS`] entries,
/// in the order the runtime returned them. An empty `data` list yields an
/// empty inventory ("serves nothing").
pub fn parse_model_list(body: &str) -> Result<Vec<ModelEntry>, serde_json::Error> {
    let list: ModelList = serde_json::from_str(body)?;
    let mut seen = HashSet::new();

    Ok(list
        .data
        .into_iter()
        .filter(|m| is_valid_model_id(&m.id))
        .filter(|m| seen.insert(m.id.clone()))
        .take(MAX_MODELS)
        .map(|m| match m.max_model_len {
            Some(len) if len > 0 => ModelEntry::Detailed {
                id: m.id,
                max_model_len: len,
            },
            _ => ModelEntry::Id(m.id),
        })
        .collect())
}
