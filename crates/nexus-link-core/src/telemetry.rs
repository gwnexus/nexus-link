use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Telemetry payload pushed to Nexus backend every interval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPayload {
    pub node_id: String,
    pub timestamp: DateTime<Utc>,
    pub system: SystemMetrics,
    pub containers: Vec<ContainerMetrics>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuMetrics>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    pub cpu_usage_percent: f64,
    pub memory_total_bytes: u64,
    pub memory_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetrics {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub cpu_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_limit_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuMetrics {
    pub devices: Vec<GpuDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuDevice {
    pub index: u32,
    pub name: String,
    pub temperature_celsius: f64,
    pub utilization_percent: f64,
    pub memory_total_bytes: u64,
    pub memory_used_bytes: u64,
    pub power_draw_watts: f64,
}
