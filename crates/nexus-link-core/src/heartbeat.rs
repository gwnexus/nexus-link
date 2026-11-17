//! Node heartbeat payload (`POST /api/nodes/{id}/heartbeat`).
//!
//! Since Nexus Hub 0.21.0 the heartbeat may carry a `models` field: the
//! inventory of models the node's local OpenAI-compatible runtime (LiteLLM
//! or vLLM) serves. The hub only uses it for "Add to gateway" suggestions.
//!
//! Rules (agreed with the hub):
//! - Each entry is a plain id string or an object
//!   `{ id, max_model_len?, backing_model? }`.
//! - `backing_model` (since 0.21.1) names the physical model behind an alias,
//!   so the hub can group LiteLLM aliases that share one runtime. It follows
//!   the id rules and is omitted when unknown. Endpoints, keys and other
//!   runtime parameters are never reported.
//! - Ids use letters, digits and `. _ : / @ + -`, at most 200 characters.
//! - At most 50 entries; invalid and duplicate ids are dropped.
//! - `models` is omitted (not sent as an empty list) when the runtime could
//!   not be queried, so a temporary failure keeps the last known inventory.
//!
//! Since 0.21.2 the heartbeat also reports `command_signatures`
//! (`enforced`, `off` or `key_missing`), so the hub can gate runtime
//! mutation on signature enforcement (see `crate::signing`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::signing::SignatureState;

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
    /// Model id with its context window and/or backing model.
    Detailed {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_model_len: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        backing_model: Option<String>,
    },
}

impl ModelEntry {
    /// Build an entry; a plain id string when no detail is known.
    ///
    /// A zero `max_model_len` and an invalid `backing_model` count as unknown.
    pub fn new(id: String, max_model_len: Option<u64>, backing_model: Option<String>) -> Self {
        let max_model_len = max_model_len.filter(|len| *len > 0);
        let backing_model = backing_model.filter(|b| is_valid_model_id(b));
        if max_model_len.is_none() && backing_model.is_none() {
            ModelEntry::Id(id)
        } else {
            ModelEntry::Detailed {
                id,
                max_model_len,
                backing_model,
            }
        }
    }

    pub fn id(&self) -> &str {
        match self {
            ModelEntry::Id(id) | ModelEntry::Detailed { id, .. } => id,
        }
    }

    pub fn max_model_len(&self) -> Option<u64> {
        match self {
            ModelEntry::Id(_) => None,
            ModelEntry::Detailed { max_model_len, .. } => *max_model_len,
        }
    }

    pub fn backing_model(&self) -> Option<&str> {
        match self {
            ModelEntry::Id(_) => None,
            ModelEntry::Detailed { backing_model, .. } => backing_model.as_deref(),
        }
    }

    /// Copy of this entry with `backing_model` set (or cleared with `None`).
    pub fn with_backing_model(self, backing_model: Option<String>) -> Self {
        let max_model_len = self.max_model_len();
        let id = match self {
            ModelEntry::Id(id) | ModelEntry::Detailed { id, .. } => id,
        };
        ModelEntry::new(id, max_model_len, backing_model)
    }
}

/// Request body of the heartbeat endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatPayload {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelEntry>>,
    /// Command signature enforcement on this node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_signatures: Option<SignatureState>,
}

impl HeartbeatPayload {
    /// An `online` heartbeat; pass `None` when the model inventory is unknown.
    pub fn online(models: Option<Vec<ModelEntry>>) -> Self {
        Self {
            status: "online".to_string(),
            models,
            command_signatures: None,
        }
    }

    /// Add the command signature enforcement state.
    pub fn with_command_signatures(mut self, state: SignatureState) -> Self {
        self.command_signatures = Some(state);
        self
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

/// LiteLLM model info URL for a runtime base URL (`<base>/model/info`).
///
/// Accepts the base with or without a trailing `/v1`, like
/// [`models_endpoint`].
pub fn model_info_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/model/info")
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
    /// vLLM: the model the served name points to (HF repo or local path).
    #[serde(default)]
    root: Option<String>,
}

/// Backing model from a vLLM `root`. Absolute local paths are not reported
/// (they reveal the node's filesystem layout and are not a model name).
fn backing_from_root(root: Option<String>) -> Option<String> {
    root.filter(|r| !r.starts_with('/') && is_valid_model_id(r))
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
        .map(|m| {
            let backing = backing_from_root(m.root);
            ModelEntry::new(m.id, m.max_model_len, backing)
        })
        .collect())
}

/// LiteLLM provider prefixes stripped from `litellm_params.model`.
const PROVIDER_PREFIXES: &[&str] = &[
    "hosted_vllm",
    "vllm",
    "openai",
    "custom_openai",
    "text-completion-openai",
    "litellm_proxy",
    "ollama",
    "ollama_chat",
    "lm_studio",
    "huggingface",
    "nvidia_nim",
    "anthropic",
    "azure",
    "azure_ai",
    "bedrock",
    "vertex_ai",
    "gemini",
    "mistral",
    "groq",
    "deepseek",
    "together_ai",
    "fireworks_ai",
    "openrouter",
    "deepinfra",
    "xai",
    "cohere",
];

/// Strip a known LiteLLM provider prefix (`hosted_vllm/Qwen/Qwen3` becomes
/// `Qwen/Qwen3`). Names without a known prefix are returned unchanged.
pub fn strip_provider_prefix(model: &str) -> &str {
    match model.split_once('/') {
        Some((prefix, rest)) if !rest.is_empty() && PROVIDER_PREFIXES.contains(&prefix) => rest,
        _ => model,
    }
}

#[derive(Deserialize)]
struct ModelInfoList {
    data: Vec<ModelInfoObject>,
}

#[derive(Deserialize)]
struct ModelInfoObject {
    model_name: String,
    #[serde(default)]
    litellm_params: Option<LitellmParams>,
}

/// Only `model` is read; `api_base`, keys and all other parameters are
/// ignored by deserialization and never leave this function.
#[derive(Deserialize)]
struct LitellmParams {
    #[serde(default)]
    model: Option<String>,
}

/// Parse a LiteLLM `GET /model/info` response into `alias -> backing model`.
///
/// An alias with several deployments maps to a backing model only when all
/// deployments agree; otherwise it is left out (unknown). Invalid names are
/// left out as well.
pub fn parse_model_info(body: &str) -> Result<HashMap<String, String>, serde_json::Error> {
    let list: ModelInfoList = serde_json::from_str(body)?;
    // None marks an alias whose deployments disagree.
    let mut map: HashMap<String, Option<String>> = HashMap::new();

    for item in list.data {
        let backing = item
            .litellm_params
            .and_then(|p| p.model)
            .map(|m| strip_provider_prefix(&m).to_string())
            .filter(|m| is_valid_model_id(m));
        let Some(backing) = backing else {
            map.insert(item.model_name, None);
            continue;
        };
        map.entry(item.model_name)
            .and_modify(|cur| {
                if cur.as_deref() != Some(backing.as_str()) {
                    *cur = None;
                }
            })
            .or_insert(Some(backing));
    }

    Ok(map
        .into_iter()
        .filter_map(|(alias, backing)| backing.map(|b| (alias, b)))
        .collect())
}

/// Fill in `backing_model` from a LiteLLM alias map. Entries that already
/// carry one (vLLM `root`) keep it.
pub fn apply_backing_models(
    models: Vec<ModelEntry>,
    info: &HashMap<String, String>,
) -> Vec<ModelEntry> {
    models
        .into_iter()
        .map(|m| {
            if m.backing_model().is_some() {
                return m;
            }
            let backing = info.get(m.id()).cloned();
            m.with_backing_model(backing)
        })
        .collect()
}
