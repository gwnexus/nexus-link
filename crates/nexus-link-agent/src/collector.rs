use bollard::Docker;
use bollard::container::StatsOptions;
use chrono::Utc;
use futures::StreamExt;
use nexus_link_core::config::Config;
use nexus_link_core::telemetry::{
    ContainerMetrics, GpuDevice, GpuMetrics, SystemMetrics, TelemetryPayload,
};
use sysinfo::{Disks, System};
use tracing::{debug, warn};

/// Collect telemetry from the local system and Docker daemon
pub async fn collect_telemetry(config: &Config) -> anyhow::Result<TelemetryPayload> {
    let system_metrics = collect_system_metrics();
    let container_metrics = collect_container_metrics().await?;
    let gpu_metrics = collect_gpu_metrics().await;
    let private_ip = detect_private_ip();

    debug!(
        containers = container_metrics.len(),
        gpu = gpu_metrics.is_some(),
        ip = ?private_ip,
        "Telemetry collected"
    );

    Ok(TelemetryPayload {
        node_id: config.node.node_id.clone(),
        timestamp: Utc::now(),
        system: system_metrics,
        containers: container_metrics,
        gpu: gpu_metrics,
        private_ip,
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
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(e) => {
            debug!("Docker not available: {}", e);
            return Ok(vec![]);
        }
    };

    let containers = docker
        .list_containers(Some(bollard::container::ListContainersOptions::<String> {
            all: false, // only running containers
            ..Default::default()
        }))
        .await?;

    let mut metrics = Vec::new();

    for container in containers {
        let id = container.id.unwrap_or_default();
        let short_id: String = id.chars().take(12).collect();
        let name = container
            .names
            .and_then(|n| n.first().cloned())
            .unwrap_or_else(|| short_id.clone())
            .trim_start_matches('/')
            .to_string();

        // Collect per-container stats (one-shot, non-streaming)
        let stats = get_container_stats(&docker, &id).await;

        metrics.push(ContainerMetrics {
            id: short_id,
            name,
            image: container.image.unwrap_or_default(),
            status: container.status.unwrap_or_default(),
            cpu_percent: stats.cpu_percent,
            memory_usage_bytes: stats.memory_usage,
            memory_limit_bytes: stats.memory_limit,
            network_rx_bytes: stats.network_rx,
            network_tx_bytes: stats.network_tx,
        });
    }

    Ok(metrics)
}

struct ContainerStats {
    cpu_percent: f64,
    memory_usage: u64,
    memory_limit: u64,
    network_rx: u64,
    network_tx: u64,
}

/// Get a single stats snapshot from a container (non-streaming)
async fn get_container_stats(docker: &Docker, container_id: &str) -> ContainerStats {
    let default = ContainerStats {
        cpu_percent: 0.0,
        memory_usage: 0,
        memory_limit: 0,
        network_rx: 0,
        network_tx: 0,
    };

    let mut stream = docker.stats(
        container_id,
        Some(StatsOptions {
            stream: false, // one-shot
            one_shot: true,
        }),
    );

    let stats = match stream.next().await {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            debug!("Stats error for {}: {}", &container_id[..12], e);
            return default;
        }
        None => return default,
    };

    // Calculate CPU percentage
    let cpu_percent = calculate_cpu_percent(&stats);

    // Memory
    let memory_usage = stats.memory_stats.usage.unwrap_or(0);
    let memory_limit = stats.memory_stats.limit.unwrap_or(0);

    // Network (sum all interfaces)
    let (network_rx, network_tx) = stats
        .networks
        .map(|nets| {
            nets.values().fold((0u64, 0u64), |(rx, tx), net| {
                (rx + net.rx_bytes, tx + net.tx_bytes)
            })
        })
        .unwrap_or((0, 0));

    ContainerStats {
        cpu_percent,
        memory_usage,
        memory_limit,
        network_rx,
        network_tx,
    }
}

/// Calculate CPU usage percentage from Docker stats
fn calculate_cpu_percent(stats: &bollard::container::Stats) -> f64 {
    let cpu_delta = stats.cpu_stats.cpu_usage.total_usage as f64
        - stats.precpu_stats.cpu_usage.total_usage as f64;

    let system_delta = stats.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
        - stats.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;

    if system_delta > 0.0 && cpu_delta > 0.0 {
        let num_cpus = stats.cpu_stats.online_cpus.unwrap_or(1) as f64;
        (cpu_delta / system_delta) * num_cpus * 100.0
    } else {
        0.0
    }
}

/// Collect GPU metrics by parsing nvidia-smi output
async fn collect_gpu_metrics() -> Option<GpuMetrics> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,temperature.gpu,utilization.gpu,memory.total,memory.used,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(_) => {
            debug!("nvidia-smi returned non-zero exit code");
            return None;
        }
        Err(e) => {
            debug!("nvidia-smi not available: {}", e);
            return None;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut devices = parse_nvidia_smi_output(&stdout);

    // For unified memory GPUs (like DGX Spark GB10), memory.total/used report [N/A].
    // Fall back to aggregating per-process GPU memory usage.
    for device in &mut devices {
        if device.memory_total_bytes == 0 && device.memory_used_bytes == 0 {
            let (used, total) = query_unified_gpu_memory().await;
            device.memory_used_bytes = used;
            device.memory_total_bytes = total;
        }
    }

    if devices.is_empty() {
        None
    } else {
        Some(GpuMetrics { devices })
    }
}

/// For unified memory systems (DGX Spark), query GPU memory from process list
/// Returns (used_bytes, total_bytes)
async fn query_unified_gpu_memory() -> (u64, u64) {
    // Get per-process GPU memory usage
    let used = query_process_gpu_memory().await;

    // Total = system memory (unified architecture shares RAM with GPU)
    let total = get_system_memory_total();

    (used, total)
}

/// Sum GPU memory used by all compute processes
async fn query_process_gpu_memory() -> u64 {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=used_gpu_memory",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .await;

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let total_mib: u64 = stdout
                .lines()
                .filter_map(|line| line.trim().parse::<u64>().ok())
                .sum();
            total_mib * 1024 * 1024
        }
        _ => 0,
    }
}

/// Read total system memory from /proc/meminfo
fn get_system_memory_total() -> u64 {
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                let kb: u64 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                return kb * 1024;
            }
        }
    }
    // Fallback: 128 GB for DGX Spark
    128 * 1024 * 1024 * 1024
}

/// Parse nvidia-smi CSV output into GpuDevice structs
///
/// Expected format (one line per GPU):
/// index, name, temperature.gpu, utilization.gpu, memory.total, memory.used, power.draw
/// 0, NVIDIA GB10 Grace Blackwell, 52, 85, 131072, 98304, 150.00
pub fn parse_nvidia_smi_output(output: &str) -> Vec<GpuDevice> {
    let mut devices = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split(',').map(|f| f.trim()).collect();
        if fields.len() < 7 {
            warn!("nvidia-smi: unexpected line format: {}", line);
            continue;
        }

        let index = fields[0].parse::<u32>().unwrap_or(0);
        let name = fields[1].to_string();
        let temperature = fields[2].parse::<f64>().unwrap_or(0.0);
        let utilization = fields[3].parse::<f64>().unwrap_or(0.0);
        // nvidia-smi reports memory in MiB, convert to bytes
        let memory_total_mib = fields[4].parse::<u64>().unwrap_or(0);
        let memory_used_mib = fields[5].parse::<u64>().unwrap_or(0);
        let power_draw = fields[6].parse::<f64>().unwrap_or(0.0);

        devices.push(GpuDevice {
            index,
            name,
            temperature_celsius: temperature,
            utilization_percent: utilization,
            memory_total_bytes: memory_total_mib * 1024 * 1024,
            memory_used_bytes: memory_used_mib * 1024 * 1024,
            power_draw_watts: power_draw,
        });
    }

    devices
}

/// Detect the primary private IP address of this machine
fn detect_private_ip() -> Option<String> {
    // Strategy: parse `ip route get 1.1.1.1` to find the source IP
    // This gives us the IP that would be used for outbound traffic
    let output = std::process::Command::new("ip")
        .args(["route", "get", "1.1.1.1"])
        .output();

    if let Ok(o) = output
        && o.status.success()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        // Output looks like: "1.1.1.1 via 10.0.0.1 dev eth0 src 10.0.0.50 uid 1000"
        if let Some(src_idx) = stdout.find("src ") {
            let after_src = &stdout[src_idx + 4..];
            let ip = after_src.split_whitespace().next().unwrap_or("");
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }

    // Fallback: parse `hostname -I` (first IP)
    let output = std::process::Command::new("hostname").arg("-I").output();

    if let Ok(o) = output
        && o.status.success()
    {
        let stdout = String::from_utf8_lossy(&o.stdout);
        let first_ip = stdout.split_whitespace().next().unwrap_or("");
        if !first_ip.is_empty() {
            return Some(first_ip.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nvidia_smi_single_gpu() {
        let output = "0, NVIDIA GB10 Grace Blackwell, 52, 85, 131072, 98304, 150.00\n";
        let devices = parse_nvidia_smi_output(output);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].index, 0);
        assert_eq!(devices[0].name, "NVIDIA GB10 Grace Blackwell");
        assert_eq!(devices[0].temperature_celsius, 52.0);
        assert_eq!(devices[0].utilization_percent, 85.0);
        assert_eq!(devices[0].memory_total_bytes, 131072 * 1024 * 1024);
        assert_eq!(devices[0].memory_used_bytes, 98304 * 1024 * 1024);
        assert_eq!(devices[0].power_draw_watts, 150.0);
    }

    #[test]
    fn test_parse_nvidia_smi_multi_gpu() {
        let output = "\
0, NVIDIA A100 80GB, 45, 92, 81920, 65536, 280.50
1, NVIDIA A100 80GB, 47, 88, 81920, 72000, 275.00
2, NVIDIA A100 80GB, 44, 95, 81920, 78000, 290.00
3, NVIDIA A100 80GB, 46, 90, 81920, 70000, 285.00
";
        let devices = parse_nvidia_smi_output(output);

        assert_eq!(devices.len(), 4);
        assert_eq!(devices[0].name, "NVIDIA A100 80GB");
        assert_eq!(devices[3].index, 3);
        assert_eq!(devices[1].temperature_celsius, 47.0);
    }

    #[test]
    fn test_parse_nvidia_smi_empty_output() {
        let devices = parse_nvidia_smi_output("");
        assert!(devices.is_empty());
    }

    #[test]
    fn test_parse_nvidia_smi_malformed_line() {
        let output = "this is not valid csv\n0, GPU, 50, 80, 16384, 8192, 100.00\n";
        let devices = parse_nvidia_smi_output(output);
        // First line skipped (not enough fields), second line parsed
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "GPU");
    }

    #[test]
    fn test_parse_nvidia_smi_dgx_spark_realistic() {
        // DGX Spark has a single integrated GPU with 128GB unified memory
        let output = "0, NVIDIA GB202, 48, 72, 131072, 94208, 125.50\n";
        let devices = parse_nvidia_smi_output(output);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].memory_total_bytes, 128 * 1024 * 1024 * 1024); // 128 GB
        assert_eq!(devices[0].power_draw_watts, 125.5);
    }
}
