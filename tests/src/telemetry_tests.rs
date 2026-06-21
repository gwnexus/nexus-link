use chrono::Utc;
use nexus_link_core::telemetry::{
    ContainerMetrics, GpuDevice, GpuMetrics, SystemMetrics, TelemetryPayload,
};

#[test]
fn test_telemetry_payload_serialization() {
    let payload = TelemetryPayload {
        node_id: "test-node".to_string(),
        timestamp: Utc::now(),
        system: SystemMetrics {
            cpu_usage_percent: 45.5,
            memory_total_bytes: 128_000_000_000,
            memory_used_bytes: 64_000_000_000,
            disk_total_bytes: 1_000_000_000_000,
            disk_used_bytes: 500_000_000_000,
            uptime_secs: 86400,
        },
        containers: vec![ContainerMetrics {
            id: "abc123def456".to_string(),
            name: "vllm-coder-main".to_string(),
            image: "vllm/vllm-openai:latest".to_string(),
            status: "running".to_string(),
            cpu_percent: 75.0,
            memory_usage_bytes: 32_000_000_000,
            memory_limit_bytes: 64_000_000_000,
            network_rx_bytes: 1_000_000,
            network_tx_bytes: 2_000_000,
        }],
        gpu: Some(GpuMetrics {
            devices: vec![GpuDevice {
                index: 0,
                name: "NVIDIA GB10 Grace Blackwell".to_string(),
                temperature_celsius: 52.0,
                utilization_percent: 85.0,
                memory_total_bytes: 128_000_000_000,
                memory_used_bytes: 96_000_000_000,
                power_draw_watts: 150.0,
            }],
        }),
    };

    let json = serde_json::to_string(&payload).unwrap();
    assert!(json.contains("test-node"));
    assert!(json.contains("vllm-coder-main"));
    assert!(json.contains("GB10 Grace Blackwell"));
}

#[test]
fn test_telemetry_payload_without_gpu() {
    let payload = TelemetryPayload {
        node_id: "node-no-gpu".to_string(),
        timestamp: Utc::now(),
        system: SystemMetrics {
            cpu_usage_percent: 10.0,
            memory_total_bytes: 16_000_000_000,
            memory_used_bytes: 8_000_000_000,
            disk_total_bytes: 500_000_000_000,
            disk_used_bytes: 100_000_000_000,
            uptime_secs: 3600,
        },
        containers: vec![],
        gpu: None,
    };

    let json = serde_json::to_string(&payload).unwrap();
    // gpu field should be omitted when None (skip_serializing_if)
    assert!(!json.contains("\"gpu\""));
}

#[test]
fn test_telemetry_payload_deserialization() {
    let json = r#"{
        "node_id": "roundtrip-test",
        "timestamp": "2026-06-21T12:00:00Z",
        "system": {
            "cpu_usage_percent": 25.0,
            "memory_total_bytes": 64000000000,
            "memory_used_bytes": 32000000000,
            "disk_total_bytes": 1000000000000,
            "disk_used_bytes": 500000000000,
            "uptime_secs": 7200
        },
        "containers": [],
        "gpu": null
    }"#;

    let payload: TelemetryPayload = serde_json::from_str(json).unwrap();
    assert_eq!(payload.node_id, "roundtrip-test");
    assert_eq!(payload.system.cpu_usage_percent, 25.0);
    assert!(payload.containers.is_empty());
    assert!(payload.gpu.is_none());
}

#[test]
fn test_system_metrics_values() {
    let metrics = SystemMetrics {
        cpu_usage_percent: 0.0,
        memory_total_bytes: 0,
        memory_used_bytes: 0,
        disk_total_bytes: 0,
        disk_used_bytes: 0,
        uptime_secs: 0,
    };
    // Should serialize without error even with zero values
    let json = serde_json::to_string(&metrics).unwrap();
    assert!(json.contains("cpu_usage_percent"));
}
