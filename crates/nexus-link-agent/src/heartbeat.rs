use std::time::Duration;

use nexus_link_core::config::Config;
use nexus_link_core::heartbeat::{
    HeartbeatPayload, HeartbeatResponse, ModelEntry, models_endpoint, parse_model_list,
};
use tracing::{info, warn};

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
    match parse_model_list(&body) {
        Ok(models) => Some(models),
        Err(e) => {
            warn!("Model list is not an OpenAI-compatible response: {}", e);
            None
        }
    }
}

/// Send one `online` heartbeat, including the model inventory when known.
pub async fn send_heartbeat(client: &reqwest::Client, config: &Config) -> anyhow::Result<()> {
    let models = fetch_models(client, config).await;
    let payload = HeartbeatPayload::online(models);

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
        "Heartbeat sent"
    );

    Ok(())
}
