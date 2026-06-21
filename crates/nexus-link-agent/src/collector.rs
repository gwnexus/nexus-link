use bollard::Docker;
use chrono::Utc;
use nexus_link_core::config::Config;
use nexus_link_core::telemetry::{ContainerMetrics, GpuMetrics, SystemMetrics, TelemetryPayload};
use sysinfo::{Disks, System};
use tracing::debug;

/// Collect telemetry from the local system and Docker daemon
pub async fn collect_telemetry(config: &Config) -> anyhow::Result<TelemetryPayload> {
    let system_metrics = collect_system_metrics();
    let container_metrics = collect_container_metrics().await?;
    let gpu_metrics = collect_gpu_metrics().await;

    debug!(
        containers = container_metrics.len(),
        gpu = gpu_metrics.is_some(),
        "Telemetry collected"
    );

    Ok(TelemetryPayload {
        node_id: config.node.node_id.clone(),
        timestamp: Utc::now(),
        system: system_metrics,
        containers: container_metrics,
        gpu: gpu_metrics,
    })
}

fn collect_system_metrics() -> SystemMetrics {
    let mut sys = System::new_all();
    sys.refresh_all();

    let disks = Disks::new_with_refreshed_list();
    let disk_total: u64 = disks.iter().map(|d| d.total_space()).sum();
    let disk_used: u64 = disks
        .iter()
        .map(|d| d.total_space() - d.available_space())
        .sum();

    SystemMetrics {
        cpu_usage_percent: sys.global_cpu_usage() as f64,
        memory_total_bytes: sys.total_memory(),
        memory_used_bytes: sys.used_memory(),
        disk_total_bytes: disk_total,
        disk_used_bytes: disk_used,
        uptime_secs: System::uptime(),
    }
}

async fn collect_container_metrics() -> anyhow::Result<Vec<ContainerMetrics>> {
    let docker = Docker::connect_with_local_defaults()?;

    let containers = docker
        .list_containers(Some(bollard::container::ListContainersOptions::<String> {
            all: false, // only running containers
            ..Default::default()
        }))
        .await?;

    let mut metrics = Vec::new();

    for container in containers {
        let id = container.id.unwrap_or_default();
        let name = container
            .names
            .and_then(|n| n.first().cloned())
            .unwrap_or_else(|| id.chars().take(12).collect())
            .trim_start_matches('/')
            .to_string();

        // TODO: Collect per-container CPU/memory stats via stats API
        // For now, provide basic info
        metrics.push(ContainerMetrics {
            id: id.chars().take(12).collect(),
            name,
            image: container.image.unwrap_or_default(),
            status: container.status.unwrap_or_default(),
            cpu_percent: 0.0,      // TODO: from stats stream
            memory_usage_bytes: 0, // TODO: from stats stream
            memory_limit_bytes: 0, // TODO: from stats stream
            network_rx_bytes: 0,   // TODO: from stats stream
            network_tx_bytes: 0,   // TODO: from stats stream
        });
    }

    Ok(metrics)
}

async fn collect_gpu_metrics() -> Option<GpuMetrics> {
    // TODO: Parse nvidia-smi output for GPU metrics on DGX Spark
    // nvidia-smi --query-gpu=index,name,temperature.gpu,utilization.gpu,memory.total,memory.used,power.draw
    //   --format=csv,noheader,nounits
    None
}
