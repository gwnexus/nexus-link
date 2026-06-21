use nexus_link_core::config::Config;
use nexus_link_core::telemetry::TelemetryPayload;
use tracing::debug;

/// Push telemetry payload to the Nexus backend
pub async fn push_telemetry(config: &Config, payload: &TelemetryPayload) -> anyhow::Result<()> {
    let url = format!(
        "{}/api/nodes/{}/telemetry",
        config.api.base_url, config.node.node_id
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(&config.node.token)
        .json(payload)
        .send()
        .await?;

    if resp.status().is_success() {
        debug!(
            node_id = %config.node.node_id,
            containers = payload.containers.len(),
            "Telemetry pushed successfully"
        );
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Telemetry push failed ({}): {}", status, body);
    }

    Ok(())
}
