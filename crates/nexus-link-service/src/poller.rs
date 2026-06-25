//! Command queue poller — ADR-0049 reverse-agent pattern.
//!
//! Polls `GET /api/nodes/:id/compose/commands/pending` at the configured
//! interval, executes each command locally using the existing compose
//! handler logic, and reports results back via
//! `PATCH /api/nodes/:id/compose/commands/:cmd_id`.
//!
//! This module runs as an independent tokio task spawned from main.rs.
//! It never receives inbound connections — all network traffic is outbound
//! to the Nexus backend, which works through any NAT/WireGuard/firewall.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use nexus_link_core::types::{
    ComposeCommandItem, ComposeCommandResult, ComposeCommandStatus, ComposeCommandType,
};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

/// One poll-and-execute cycle. Called on every tick of the poll interval.
pub async fn poll_and_execute(state: &Arc<AppState>) -> anyhow::Result<()> {
    let config = &state.config;

    let Some(ref cmd_token) = config.compose.cmd_token else {
        return Ok(());
    };

    let base_url = &config.api.base_url;
    let node_id = &config.node.node_id;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    // ── Fetch pending commands ──────────────────────────────────────────────
    let resp = client
        .get(format!(
            "{}/api/nodes/{}/compose/commands/pending",
            base_url, node_id
        ))
        .bearer_auth(cmd_token)
        .send()
        .await?;

    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(());
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(status = %status, "Pending commands poll returned error: {}", body);
        return Ok(());
    }

    let commands: Vec<ComposeCommandItem> = resp.json().await?;

    if commands.is_empty() {
        return Ok(());
    }

    info!(count = commands.len(), "Received compose commands");

    // ── Execute each command sequentially ──────────────────────────────────
    for cmd in commands {
        debug!(id = %cmd.id, type_ = ?cmd.command_type, "Executing command");

        let result = execute_command(&cmd, state).await;

        // Report result back to Nexus backend
        let patch_resp = client
            .patch(format!(
                "{}/api/nodes/{}/compose/commands/{}",
                base_url, node_id, cmd.id
            ))
            .bearer_auth(cmd_token)
            .json(&result)
            .send()
            .await;

        match patch_resp {
            Ok(r) if r.status().is_success() => {
                info!(id = %cmd.id, status = ?result.status, "Command result reported");
            }
            Ok(r) => {
                warn!(id = %cmd.id, status = %r.status(), "Failed to report command result");
            }
            Err(e) => {
                error!(id = %cmd.id, "Failed to send command result: {}", e);
            }
        }
    }

    Ok(())
}

/// Execute a single queued command and return the result payload.
async fn execute_command(cmd: &ComposeCommandItem, state: &Arc<AppState>) -> ComposeCommandResult {
    match cmd.command_type {
        ComposeCommandType::GetFile => execute_get_file(state),
        ComposeCommandType::PutFile => execute_put_file(cmd, state),
        ComposeCommandType::Activate => execute_activate(state).await,
        ComposeCommandType::GetLogsSnapshot => execute_logs_snapshot(cmd, state).await,
    }
}

// ── GetFile ─────────────────────────────────────────────────────────────────

fn execute_get_file(state: &Arc<AppState>) -> ComposeCommandResult {
    let compose_dir = &state.config.compose.dir;

    let compose_path = find_compose_file(compose_dir);
    let Some(ref path) = compose_path else {
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!(
                "No docker-compose.yaml found in {}",
                compose_dir.display()
            )),
        };
    };

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return ComposeCommandResult {
                status: ComposeCommandStatus::Failed,
                result: None,
                error: Some(format!("Failed to read compose file: {}", e)),
            };
        }
    };

    // Extra config files
    let extra_files: Vec<serde_json::Value> =
        read_extra_files(compose_dir, &state.config.compose.extra_extensions)
            .into_iter()
            .map(
                |(name, file_content)| serde_json::json!({ "name": name, "content": file_content }),
            )
            .collect();

    ComposeCommandResult {
        status: ComposeCommandStatus::Completed,
        result: Some(serde_json::json!({
            "path": path.display().to_string(),
            "content": content,
            "files": extra_files,
        })),
        error: None,
    }
}

// ── PutFile ─────────────────────────────────────────────────────────────────

fn execute_put_file(cmd: &ComposeCommandItem, state: &Arc<AppState>) -> ComposeCommandResult {
    let content = match cmd.args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return ComposeCommandResult {
                status: ComposeCommandStatus::Failed,
                result: None,
                error: Some("Missing 'content' in args".to_string()),
            };
        }
    };

    // Validate YAML
    if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Invalid YAML: {}", e)),
        };
    }

    let compose_dir = &state.config.compose.dir;
    if let Err(e) = std::fs::create_dir_all(compose_dir) {
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Cannot create compose directory: {}", e)),
        };
    }

    let compose_path = compose_dir.join("docker-compose.yaml");
    let tmp_path = compose_dir.join("docker-compose.yaml.tmp");

    if let Err(e) = std::fs::write(&tmp_path, &content) {
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Failed to write tmp file: {}", e)),
        };
    }

    if let Err(e) = std::fs::rename(&tmp_path, &compose_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Failed to commit file: {}", e)),
        };
    }

    ComposeCommandResult {
        status: ComposeCommandStatus::Completed,
        result: Some(serde_json::json!({
            "path": compose_path.display().to_string(),
            "committed": false,
        })),
        error: None,
    }
}

// ── Activate ────────────────────────────────────────────────────────────────

async fn execute_activate(state: &Arc<AppState>) -> ComposeCommandResult {
    let compose_dir = state.config.compose.dir.clone();

    let run = timeout(
        Duration::from_secs(120),
        Command::new("docker")
            .args(["compose", "up", "-d"])
            .current_dir(&compose_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match run {
        Err(_) => ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some("Timeout: docker compose up -d did not complete within 120s".to_string()),
        },
        Ok(Err(e)) => ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Failed to spawn docker compose: {}", e)),
        },
        Ok(Ok(output)) => {
            let success = output.status.success();
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            ComposeCommandResult {
                status: if success {
                    ComposeCommandStatus::Completed
                } else {
                    ComposeCommandStatus::Failed
                },
                result: Some(serde_json::json!({
                    "success": success,
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                })),
                error: if success {
                    None
                } else {
                    Some(format!("exit code {}", exit_code))
                },
            }
        }
    }
}

// ── GetLogsSnapshot ─────────────────────────────────────────────────────────

async fn execute_logs_snapshot(
    cmd: &ComposeCommandItem,
    state: &Arc<AppState>,
) -> ComposeCommandResult {
    let tail = cmd
        .args
        .get("tail")
        .and_then(|v| v.as_u64())
        .unwrap_or(200)
        .to_string();
    let service = cmd.args.get("service").and_then(|v| v.as_str());

    let compose_dir = state.config.compose.dir.clone();

    let mut args = vec![
        "compose".to_string(),
        "logs".to_string(),
        "--no-color".to_string(),
        "--tail".to_string(),
        tail,
    ];
    if let Some(svc) = service {
        args.push(svc.to_string());
    }

    let run = timeout(
        Duration::from_secs(15),
        Command::new("docker")
            .args(&args)
            .current_dir(&compose_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match run {
        Err(_) => ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some("Timeout collecting log snapshot".to_string()),
        },
        Ok(Err(e)) => ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some(format!("Failed to run docker compose logs: {}", e)),
        },
        Ok(Ok(output)) => {
            let combined = String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr);
            let lines: Vec<&str> = combined.lines().collect();
            ComposeCommandResult {
                status: ComposeCommandStatus::Completed,
                result: Some(serde_json::json!({
                    "service": service,
                    "lines": lines,
                })),
                error: None,
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn find_compose_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let yaml = dir.join("docker-compose.yaml");
    if yaml.exists() {
        return Some(yaml);
    }
    let yml = dir.join("docker-compose.yml");
    if yml.exists() {
        return Some(yml);
    }
    None
}

fn read_extra_files(dir: &std::path::Path, extensions: &[String]) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut files = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name == "docker-compose.yaml" || name == "docker-compose.yml" {
            continue;
        }
        if !extensions.iter().any(|ext| name.ends_with(ext.as_str())) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            files.push((name, content));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}
