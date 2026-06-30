use nexus_link_core::config::{
    AgentConfig, ApiConfig, ApiTokens, ComposeConfig, Config, NodeConfig, ServiceConfig, TokenEntry,
};
use tempfile::TempDir;

fn make_api_config(base_url: &str) -> ApiConfig {
    ApiConfig {
        base_url: base_url.to_string(),
        tokens: ApiTokens {
            telemetry: Some(TokenEntry {
                token: "nxs_node_testtoken".to_string(),
                scope: "read".to_string(),
            }),
            command: None,
        },
        push_interval_secs: None,
    }
}

#[test]
fn test_config_save_and_load() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let config = Config {
        node: NodeConfig {
            node_id: "test-node-123".to_string(),
            name: "test-node".to_string(),
            token: "nxs_node_testtoken".to_string(),
            tags: vec!["gpu".to_string(), "aarch64".to_string()],
        },
        api: make_api_config("https://nexus.gatewarden.eu"),
        agent: AgentConfig {
            push_sec: 30,
            poll_sec: 2,
        },
        service: ServiceConfig::default(),
        compose: ComposeConfig::default(),
    };

    config.save_to(config_path.clone()).unwrap();
    let loaded = Config::load_from(config_path).unwrap();

    assert_eq!(loaded.node.node_id, "test-node-123");
    assert_eq!(loaded.node.name, "test-node");
    assert_eq!(loaded.node.token, "nxs_node_testtoken");
    assert_eq!(loaded.node.tags, vec!["gpu", "aarch64"]);
    assert_eq!(loaded.api.base_url, "https://nexus.gatewarden.eu");
    assert_eq!(loaded.agent.push_sec, 30);
}

#[test]
fn test_config_default_service() {
    let svc = ServiceConfig::default();
    assert_eq!(svc.listen_addr, "127.0.0.1");
    assert_eq!(svc.port, 8443);
}

#[test]
fn test_config_default_agent() {
    let agent = AgentConfig::default();
    assert_eq!(agent.push_sec, 6);
    assert_eq!(agent.poll_sec, 2);
}

#[test]
fn test_config_default_compose() {
    let compose = ComposeConfig::default();
    assert_eq!(compose.dir.to_str().unwrap(), "/opt/dgx-llm");
    assert!(compose.extra_extensions.contains(&".env".to_string()));
}

#[test]
fn test_config_load_missing_file() {
    let result = Config::load_from("/nonexistent/path/config.toml".into());
    assert!(result.is_err());
}

#[test]
fn test_config_save_creates_directory() {
    let tmp = TempDir::new().unwrap();
    let nested_path = tmp.path().join("nested").join("dir").join("config.toml");

    let config = Config {
        node: NodeConfig {
            node_id: "id".to_string(),
            name: "name".to_string(),
            token: "nxs_node_tok".to_string(),
            tags: vec![],
        },
        api: make_api_config("https://example.com"),
        agent: AgentConfig {
            push_sec: 60,
            poll_sec: 2,
        },
        service: ServiceConfig::default(),
        compose: ComposeConfig::default(),
    };

    config.save_to(nested_path.clone()).unwrap();
    assert!(nested_path.exists());
}

#[test]
fn test_config_roundtrip_with_custom_service() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let config = Config {
        node: NodeConfig {
            node_id: "node-1".to_string(),
            name: "spark".to_string(),
            token: "nxs_node_x".to_string(),
            tags: vec![],
        },
        api: make_api_config("https://api.example.com"),
        agent: AgentConfig {
            push_sec: 15,
            poll_sec: 3,
        },
        service: ServiceConfig {
            listen_addr: "127.0.0.1".to_string(),
            port: 9443,
        },
        compose: ComposeConfig::default(),
    };

    config.save_to(config_path.clone()).unwrap();
    let loaded = Config::load_from(config_path).unwrap();

    assert_eq!(loaded.service.listen_addr, "127.0.0.1");
    assert_eq!(loaded.service.port, 9443);
    assert_eq!(loaded.agent.push_sec, 15);
    assert_eq!(loaded.agent.poll_sec, 3);
}

#[test]
fn test_config_roundtrip_with_custom_compose_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let config = Config {
        node: NodeConfig {
            node_id: "node-2".to_string(),
            name: "spark2".to_string(),
            token: "nxs_node_y".to_string(),
            tags: vec![],
        },
        api: ApiConfig {
            base_url: "https://api.example.com".to_string(),
            tokens: ApiTokens {
                telemetry: Some(TokenEntry {
                    token: "nxs_node_y".to_string(),
                    scope: "read".to_string(),
                }),
                command: Some(TokenEntry {
                    token: "nxs_cmd_testtoken".to_string(),
                    scope: "read_write".to_string(),
                }),
            },
            push_interval_secs: None,
        },
        agent: AgentConfig::default(),
        service: ServiceConfig::default(),
        compose: ComposeConfig {
            dir: std::path::PathBuf::from("/srv/llm-stack"),
            extra_extensions: vec![".env".into()],
            cmd_token: Some("nxs_cmd_testtoken".to_string()),
            signing_public_key: None,
            require_signatures: false,
        },
    };

    config.save_to(config_path.clone()).unwrap();
    let loaded = Config::load_from(config_path).unwrap();

    assert_eq!(loaded.compose.dir.to_str().unwrap(), "/srv/llm-stack");
    assert_eq!(loaded.compose.extra_extensions, vec![".env"]);
    assert_eq!(loaded.agent.push_sec, 6); // default
    assert_eq!(loaded.agent.poll_sec, 2); // default
}

#[test]
fn test_config_migration_legacy_push_interval() {
    // Verify that old TOML with api.push_interval_secs is migrated to agent.push_sec
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let legacy_toml = r#"
[node]
node_id = "legacy-node"
name    = "old-spark"
token   = "nxs_node_legacytoken"

[api]
base_url           = "https://nexus.gatewarden.eu"
push_interval_secs = 20

[service]
listen_addr = "0.0.0.0"
port        = 8443
"#;

    std::fs::write(&config_path, legacy_toml).unwrap();
    let loaded = Config::load_from(config_path).unwrap();

    // Migration: push_interval_secs → agent.push_sec
    assert_eq!(loaded.agent.push_sec, 20);
    // Legacy field should be cleared after migration
    assert!(loaded.api.push_interval_secs.is_none());
    // node.token → api.tokens.telemetry
    assert_eq!(
        loaded.api.tokens.telemetry.as_ref().unwrap().token,
        "nxs_node_legacytoken"
    );
}

#[test]
fn test_config_node_token_accessor() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let config = Config {
        node: NodeConfig {
            node_id: "n1".to_string(),
            name: "n".to_string(),
            token: "nxs_node_fallback".to_string(),
            tags: vec![],
        },
        api: ApiConfig {
            base_url: "https://nexus.gatewarden.eu".to_string(),
            tokens: ApiTokens {
                telemetry: Some(TokenEntry {
                    token: "nxs_node_primary".to_string(),
                    scope: "read".to_string(),
                }),
                command: None,
            },
            push_interval_secs: None,
        },
        agent: AgentConfig::default(),
        service: ServiceConfig::default(),
        compose: ComposeConfig::default(),
    };

    config.save_to(config_path).unwrap();
    // node_token() prefers api.tokens.telemetry
    assert_eq!(config.node_token(), "nxs_node_primary");
}
