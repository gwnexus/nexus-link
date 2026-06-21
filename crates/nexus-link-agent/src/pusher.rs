use nexus_link_core::config::Config;
use nexus_link_core::telemetry::TelemetryPayload;
use tracing::info;

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
        // Log a compact summary of what was pushed
        let gpu_summary = payload.gpu.as_ref().map(|g| {
            g.devices
                .iter()
                .map(|d| {
                    format!(
                        "{}({}% util, {}/{}GB VRAM, {}W, {}C)",
                        d.name.split_whitespace().last().unwrap_or(&d.name),
                        d.utilization_percent as u32,
                        d.memory_used_bytes / (1024 * 1024 * 1024),
                        d.memory_total_bytes / (1024 * 1024 * 1024),
                        d.power_draw_watts as u32,
                        d.temperature_celsius as u32,
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        });

        info!(
            node_id = %config.node.node_id,
            cpu = format!("{:.1}%", payload.system.cpu_usage_percent),
            mem = format!("{}/{}GB",
                payload.system.memory_used_bytes / (1024 * 1024 * 1024),
                payload.system.memory_total_bytes / (1024 * 1024 * 1024)),
            containers = payload.containers.len(),
            gpu = gpu_summary.as_deref().unwrap_or("none"),
            ip = payload.private_ip.as_deref().unwrap_or("unknown"),
            "Telemetry pushed"
        );
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Telemetry push failed ({}): {}", status, body);
    }

    Ok(())
}
