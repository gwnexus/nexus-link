use std::collections::HashMap;
use std::time::Duration;

use nexus_link_core::config::{Config, dirs_home};
use nexus_link_core::heartbeat::{
    HeartbeatPayload, HeartbeatResponse, ModelEntry, apply_backing_models, model_info_endpoint,
    models_endpoint, parse_model_info, parse_model_list,
};
use nexus_link_core::signing::{SignatureState, load_verifying_key, signature_state};
use tracing::{debug, info, warn};

/// Timeout for the local runtime's model list.
const MODELS_TIMEOUT: Duration = Duration::from_secs(5);

/// Query the local runtime for the models it serves.
///
/// Returns `None` when no runtime is configured or it cannot be queried, so
/// the heartbeat omits `models` and the hub keeps the last known inventory.
pub async fn fetch_models(client: &reqwest::Client, config: &Config) -> Option<Vec<ModelEntry>> {
    let base = config.agent.models_url.as_deref()?;

    let mut req = client.get(models_endpoint(base)).timeout(MODELS_TIMEOUT);
    if let Some(key) = config.agent.models_api_key.as_deref() {
        req = req.bearer_auth(key);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Model list unavailable: {}", e.without_url());
            return None;
        }
    };
    if !resp.status().is_success() {
        warn!(status = %resp.status(), "Model list request rejected by runtime");
        return None;
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Model list unreadable: {}", e.without_url());
            return None;
        }
    };
    let models = match parse_model_list(&body) {
        Ok(models) => models,
        Err(e) => {
            warn!("Model list is not an OpenAI-compatible response: {}", e);
            return None;
        }
    };

    // vLLM reports the backing model as `root` in /v1/models. LiteLLM does
    // not; ask its /model/info for the aliases instead.
    if models.is_empty() || models.iter().any(|m| m.backing_model().is_some()) {
        return Some(models);
    }
    match fetch_model_info(client, config, base).await {
        Some(info) => Some(apply_backing_models(models, &info)),
        None => Some(models),
    }
}

/// Query LiteLLM's `/model/info` for `alias -> backing model`.
///
/// Returns `None` when the endpoint is missing (plain vLLM), not readable with
/// the configured key, or not a LiteLLM response. The heartbeat then reports
/// the models without `backing_model`.
async fn fetch_model_info(
    client: &reqwest::Client,
    config: &Config,
    base: &str,
) -> Option<HashMap<String, String>> {
    let mut req = client
        .get(model_info_endpoint(base))
        .timeout(MODELS_TIMEOUT);
    if let Some(key) = config.agent.models_api_key.as_deref() {
        req = req.bearer_auth(key);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            debug!("Model info unavailable: {}", e.without_url());
            return None;
        }
    };
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        debug!("Runtime has no /model/info; backing models unknown");
        return None;
    }
    if !status.is_success() {
        warn!(%status, "Model info rejected by runtime; check that agent.models_api_key may read /model/info");
        return None;
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            debug!("Model info unreadable: {}", e.without_url());
            return None;
        }
    };
    match parse_model_info(&body) {
        Ok(info) => Some(info),
        Err(e) => {
            debug!("Model info is not a LiteLLM response: {}", e);
            None
        }
    }
}

/// Command signature enforcement as the service applies it: same config
/// flag and same key sources (`signing_key.pub`, then config).
pub fn command_signature_state(config: &Config) -> SignatureState {
    let key = load_verifying_key(&config.compose, &dirs_home());
    signature_state(config.compose.require_signatures, &key)
}

/// Send one `online` heartbeat, including the model inventory when known
/// and the command signature enforcement state.
pub async fn send_heartbeat(client: &reqwest::Client, config: &Config) -> anyhow::Result<()> {
    let models = fetch_models(client, config).await;
    let signatures = command_signature_state(config);
    let payload = HeartbeatPayload::online(models).with_command_signatures(signatures);

    let url = format!(
        "{}/api/nodes/{}/heartbeat",
        config.api.base_url, config.node.node_id
    );
    let resp = client
        .post(&url)
        .bearer_auth(config.node_token())
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Heartbeat failed ({}): {}", status, body);
    }

    let response: HeartbeatResponse = resp.json().await.unwrap_or_default();
    info!(
        node_id = %config.node.node_id,
        models_sent = payload.models.as_ref().map(|m| m.len()),
        models_recorded = response.models_recorded,
        command_signatures = ?signatures,
        "Heartbeat sent"
    );

    Ok(())
}
