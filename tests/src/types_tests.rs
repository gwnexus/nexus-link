use nexus_link_core::types::{
    CommandResponse, ComposeLogsPayload, ComposeRestartPayload, ConfigExchangePayload, NodeCommand,
    RegisterRequest, RegisterResponse,
};

#[test]
fn test_node_command_compose_restart_serialization() {
    let cmd = NodeCommand::ComposeRestart(ComposeRestartPayload {
        service: Some("vllm-coder".to_string()),
    });

    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("compose_restart"));
    assert!(json.contains("vllm-coder"));
}

#[test]
fn test_node_command_compose_restart_all() {
    let cmd = NodeCommand::ComposeRestart(ComposeRestartPayload { service: None });

    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("compose_restart"));
}

#[test]
fn test_node_command_compose_logs_serialization() {
    let cmd = NodeCommand::ComposeLogs(ComposeLogsPayload {
        service: "litellm".to_string(),
        tail: 200,
    });

    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("compose_logs"));
    assert!(json.contains("litellm"));
    assert!(json.contains("200"));
}

#[test]
fn test_node_command_config_exchange_serialization() {
    let cmd = NodeCommand::ConfigExchange(ConfigExchangePayload {
        key: "agent.interval".to_string(),
        value: Some(serde_json::json!(60)),
    });

    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("config_exchange"));
    assert!(json.contains("agent.interval"));
}

#[test]
fn test_node_command_deserialization() {
    let json = r#"{"type":"compose_restart","payload":{"service":"caddy"}}"#;
    let cmd: NodeCommand = serde_json::from_str(json).unwrap();

    match cmd {
        NodeCommand::ComposeRestart(payload) => {
            assert_eq!(payload.service, Some("caddy".to_string()));
        }
        _ => panic!("Expected ComposeRestart"),
    }
}

#[test]
fn test_command_response_success() {
    let resp = CommandResponse {
        success: true,
        message: "Restarted".to_string(),
        data: None,
    };

    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("true"));
    assert!(json.contains("Restarted"));
    // data should be omitted when None
    assert!(!json.contains("\"data\""));
}

#[test]
fn test_command_response_with_data() {
    let resp = CommandResponse {
        success: true,
        message: "Logs fetched".to_string(),
        data: Some(serde_json::json!({"lines": ["line1", "line2"]})),
    };

    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"data\""));
    assert!(json.contains("line1"));
}

#[test]
fn test_register_request_serialization() {
    let req = RegisterRequest {
        name: "dgx-spark".to_string(),
        private_ip: Some("10.0.0.50".to_string()),
        tags: vec!["gpu".to_string(), "aarch64".to_string()],
        description: Some("DGX Spark node".to_string()),
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("dgx-spark"));
    assert!(json.contains("10.0.0.50"));
    assert!(json.contains("gpu"));
}

#[test]
fn test_register_response_deserialization() {
    let json = r#"{"node_id":"uuid-1234","token":"nxs_node_abc"}"#;
    let resp: RegisterResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.node_id, "uuid-1234");
    assert_eq!(resp.token, "nxs_node_abc");
}

#[test]
fn test_compose_logs_default_tail() {
    let json = r#"{"service":"vllm"}"#;
    let payload: ComposeLogsPayload = serde_json::from_str(json).unwrap();
    assert_eq!(payload.tail, 100); // default_tail_lines
}
