//! Command queue poller — ADR-0049 reverse-agent pattern.
//!
//! Polls `GET /api/nodes/:id/compose/commands/pending` at the configured
//! interval, executes each command locally using the existing compose
//! handler logic, and reports results back via
//! `PATCH /api/nodes/:id/compose/commands/:cmd_id`.
//!
//! Every command passes Ed25519 verification first (`nexus_link_core::signing`).
//! With `compose.require_signatures = true` unsigned or invalid commands are
//! rejected, reported to the backend as failed and never executed.
//!
//! This module runs as an independent tokio task spawned from main.rs.
//! It never receives inbound connections — all network traffic is outbound
//! to the Nexus backend, which works through any NAT/WireGuard/firewall.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use nexus_link_core::config::COMPOSE_READ_ONLY_ERROR;
use nexus_link_core::signing::{NonceCache, VerifyContext, verify_queued_command};
use nexus_link_core::types::{
    ComposeCommandItem, ComposeCommandResult, ComposeCommandStatus, ComposeCommandType,
};
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::compose_logs;
use crate::state::AppState;

/// Maximum output length returned to the backend (SEC-008).
const MAX_OUTPUT_LEN: usize = 8192;

/// Truncate command output to prevent information disclosure (SEC-008).
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_LEN {
        s.to_string()
    } else {
        // Cut at a char boundary; slicing at a fixed byte index panics on
        // multi-byte UTF-8 (e.g. progress bars in docker output).
        format!(
            "{}... [truncated, {} bytes total]",
            &s[..s.floor_char_boundary(MAX_OUTPUT_LEN)],
            s.len()
        )
    }
}

/// Everything command verification and execution need, decoupled from
/// `AppState` (which also holds a Docker client) so it can be tested.
pub struct CommandContext<'a> {
    pub node_id: &'a str,
    pub compose_dir: PathBuf,
    pub extra_extensions: &'a [String],
    pub require_signatures: bool,
    pub signing_keys: &'a [VerifyingKey],
    pub nonces: &'a NonceCache,
    /// `compose.writable`; `false` rejects `put_file` and `activate`.
    pub compose_writable: bool,
}

impl<'a> CommandContext<'a> {
    pub fn from_state(state: &'a AppState) -> Self {
        Self {
            node_id: &state.config.node.node_id,
            compose_dir: state.config.compose.dir.clone(),
            extra_extensions: &state.config.compose.extra_extensions,
            require_signatures: state.config.compose.require_signatures,
            signing_keys: &state.signing_pubkeys,
            nonces: &state.nonces,
            compose_writable: state.config.compose.writable,
        }
    }
}

/// One poll-and-execute cycle. Called on every tick of the poll interval.
/// Returns the suggested next poll interval from the `X-Poll-Interval` header
/// (in seconds), or `None` if the header is absent.
pub async fn poll_and_execute(state: &Arc<AppState>) -> anyhow::Result<Option<u64>> {
    let config = &state.config;

    let Some(ref cmd_token) = config.compose.cmd_token else {
        return Ok(None);
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

    // Parse X-Poll-Interval header
    let poll_interval_hint = resp
        .headers()
        .get("x-poll-interval")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(poll_interval_hint);
    }

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!(status = %status, "Pending commands poll returned error: {}", body);
        return Ok(poll_interval_hint);
    }

    // Items are parsed one by one: a single malformed item must not block
    // every other command on this node.
    let items: Vec<serde_json::Value> = resp.json().await?;

    if items.is_empty() {
        return Ok(poll_interval_hint);
    }

    info!(count = items.len(), "Received compose commands");

    let ctx = CommandContext::from_state(state);

    // ── Verify and execute each command sequentially ───────────────────────
    for item in items {
        let (id, result) = match parse_item(item) {
            ParsedItem::Command(cmd) => {
                let result = handle_command(&cmd, &ctx).await;
                (cmd.id, result)
            }
            ParsedItem::Malformed { id, error } => {
                warn!(id = %id, error = %error, "Rejected malformed queued command");
                (id, failed(error))
            }
            ParsedItem::NoId => {
                warn!("Skipping queued command without an id; cannot report a result");
                continue;
            }
        };

        // Report result back to Nexus backend
        let patch_resp = client
            .patch(format!(
                "{}/api/nodes/{}/compose/commands/{}",
                base_url, node_id, id
            ))
            .bearer_auth(cmd_token)
            .json(&result)
            .send()
            .await;

        match patch_resp {
            Ok(r) if r.status().is_success() => {
                info!(id = %id, status = ?result.status, "Command result reported");
            }
            Ok(r) => {
                warn!(id = %id, status = %r.status(), "Failed to report command result");
            }
            Err(e) => {
                error!(id = %id, "Failed to send command result: {}", e);
            }
        }
    }

    Ok(poll_interval_hint)
}

/// One entry of the pending list after parsing.
#[derive(Debug)]
pub enum ParsedItem {
    Command(ComposeCommandItem),
    /// Not a valid command item, but it has an id, so the failure can be
    /// reported to the hub.
    Malformed {
        id: String,
        error: String,
    },
    /// No usable id: nothing to report to.
    NoId,
}

/// Parse one pending item. Unknown command types parse fine (see
/// `ComposeCommandType::Unknown`); this only catches structurally broken items.
pub fn parse_item(item: serde_json::Value) -> ParsedItem {
    let id = item
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    match (serde_json::from_value::<ComposeCommandItem>(item), id) {
        (Ok(cmd), Some(_)) => ParsedItem::Command(cmd),
        (Err(e), Some(id)) => ParsedItem::Malformed {
            id,
            error: format!("malformed command: {e}"),
        },
        (_, None) => ParsedItem::NoId,
    }
}

fn failed(error: String) -> ComposeCommandResult {
    ComposeCommandResult {
        status: ComposeCommandStatus::Failed,
        result: None,
        error: Some(error),
    }
}

/// Verify a queued command and execute it only if verification passes.
/// A rejected command yields a failed result whose error starts with
/// `signature:`; nothing is executed.
pub async fn handle_command(
    cmd: &ComposeCommandItem,
    ctx: &CommandContext<'_>,
) -> ComposeCommandResult {
    let verify_ctx = VerifyContext {
        node_id: ctx.node_id,
        require_signatures: ctx.require_signatures,
        keys: ctx.signing_keys,
        nonces: ctx.nonces,
        now: chrono::Utc::now(),
    };

    let verified = match verify_queued_command(cmd, &verify_ctx) {
        Ok(v) => v,
        Err(e) => {
            warn!(id = %cmd.id, type_ = cmd.command_type.as_str(), error = %e, "Rejected queued command");
            return ComposeCommandResult {
                status: ComposeCommandStatus::Failed,
                result: None,
                error: Some(e.to_string()),
            };
        }
    };

    if !verified.signed {
        debug!(id = %cmd.id, "Executing unsigned command (signature enforcement is off)");
    }
    debug!(id = %cmd.id, type_ = cmd.command_type.as_str(), signed = verified.signed, "Executing command");

    // Checked after verification, so a forged write still reports `signature:`.
    let is_write = matches!(
        cmd.command_type,
        ComposeCommandType::PutFile | ComposeCommandType::Activate
    );
    if is_write && !ctx.compose_writable {
        warn!(id = %cmd.id, type_ = cmd.command_type.as_str(), "Rejected compose write on a read-only node");
        return failed(COMPOSE_READ_ONLY_ERROR.to_string());
    }

    let args = &verified.args;
    match &cmd.command_type {
        ComposeCommandType::GetFile => execute_get_file(&ctx.compose_dir, ctx.extra_extensions),
        ComposeCommandType::PutFile => execute_put_file(args, &ctx.compose_dir),
        ComposeCommandType::Activate => execute_activate(&ctx.compose_dir).await,
        ComposeCommandType::GetLogsSnapshot => execute_logs_snapshot(args, &ctx.compose_dir).await,
        ComposeCommandType::ValidateYaml => execute_validate_yaml(args),
        ComposeCommandType::Unknown(name) => {
            warn!(id = %cmd.id, type_ = name.as_str(), "Rejected unsupported command type");
            failed(format!("unsupported command type: {name}"))
        }
    }
}

// ── GetFile ─────────────────────────────────────────────────────────────────

fn execute_get_file(compose_dir: &Path, extra_extensions: &[String]) -> ComposeCommandResult {
    let compose_path = find_compose_file(compose_dir);
    let Some(ref path) = compose_path else {
        return ComposeCommandResult {
            status: ComposeCommandStatus::Failed,
            result: None,
            error: Some("No docker-compose.yaml found in compose directory".to_string()),
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
    let extra_files: Vec<serde_json::Value> = read_extra_files(compose_dir, extra_extensions)
        .into_iter()
        .map(|(name, file_content)| serde_json::json!({ "name": name, "content": file_content }))
        .collect();

    ComposeCommandResult {
        status: ComposeCommandStatus::Completed,
        result: Some(serde_json::json!({
            "path": path.file_name().unwrap_or_default().to_string_lossy(),
            "content": content,
            "files": extra_files,
        })),
        error: None,
    }
}

// ── PutFile ─────────────────────────────────────────────────────────────────

fn execute_put_file(args: &serde_json::Value, compose_dir: &Path) -> ComposeCommandResult {
    let content = match args.get("content").and_then(|v| v.as_str()) {
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
            "path": "docker-compose.yaml",
            "committed": false,
        })),
        error: None,
    }
}

// ── Activate ────────────────────────────────────────────────────────────────

async fn execute_activate(compose_dir: &Path) -> ComposeCommandResult {
    let run = timeout(
        Duration::from_secs(120),
        Command::new("docker")
            .args(["compose", "up", "-d"])
            .current_dir(compose_dir)
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
            // SEC-008: Truncate command output to prevent information disclosure
            let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
            let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));
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

/// Log snapshot through `docker logs` per container of the stack, so
/// `.env` and the compose file are never read (see `compose_logs`).
async fn execute_logs_snapshot(
    args: &serde_json::Value,
    compose_dir: &Path,
) -> ComposeCommandResult {
    let tail = args
        .get("tail")
        .and_then(|v| v.as_u64())
        .unwrap_or(100)
        .min(10_000);
    let service = args.get("service").and_then(|v| v.as_str());

    if let Some(svc) = service {
        // SEC-003: only plain service names; the value is compared against
        // container labels and never passed to docker as an argument.
        let valid = !svc.is_empty()
            && !svc.starts_with('-')
            && svc
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return ComposeCommandResult {
                status: ComposeCommandStatus::Failed,
                result: None,
                error: Some(format!("Invalid service name: '{}'", svc)),
            };
        }
    }

    let req = compose_logs::SnapshotRequest {
        compose_dir,
        service,
        tail,
        // Without a service, --tail applies per container; limit the range
        // so the snapshot stays small and fast.
        since: service.is_none().then_some("5m"),
        timeout: Duration::from_secs(15),
    };

    match compose_logs::snapshot(&req).await {
        Ok(snap) => ComposeCommandResult {
            status: ComposeCommandStatus::Completed,
            result: Some(serde_json::json!({
                "service": service,
                "lines": snap.lines,
                "truncated": snap.truncated,
                "containers": snap.containers,
            })),
            error: None,
        },
        Err(e) => {
            warn!(error = %e, "Log snapshot failed");
            ComposeCommandResult {
                status: ComposeCommandStatus::Failed,
                result: None,
                error: Some(e),
            }
        }
    }
}

// ── ValidateYaml ────────────────────────────────────────────────────────────

/// Dry-run YAML validation — parses content with the same serde_yaml parser
/// used by `put_file`. Returns `valid: true/false` without writing any file.
fn execute_validate_yaml(args: &serde_json::Value) -> ComposeCommandResult {
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return ComposeCommandResult {
                status: ComposeCommandStatus::Completed,
                result: Some(
                    serde_json::json!({ "valid": false, "error": "Missing 'content' in args" }),
                ),
                error: None,
            };
        }
    };

    match serde_yaml::from_str::<serde_yaml::Value>(content) {
        Ok(_) => ComposeCommandResult {
            status: ComposeCommandStatus::Completed,
            result: Some(serde_json::json!({ "valid": true })),
            error: None,
        },
        Err(e) => ComposeCommandResult {
            status: ComposeCommandStatus::Completed,
            result: Some(serde_json::json!({ "valid": false, "error": e.to_string() })),
            error: None,
        },
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

const EXTENSIONLESS_ALLOWLIST: &[&str] = &["Caddyfile", "Dockerfile", "Makefile"];

/// Read extra config files from the compose directory and one level of subdirectories.
/// Returns (relative_name, content) tuples.
fn read_extra_files(dir: &std::path::Path, extensions: &[String]) -> Vec<(String, String)> {
    let Ok(dir_canonical) = dir.canonicalize() else {
        return vec![];
    };
    let mut files = vec![];
    collect_poller_files(dir, dir, extensions, &dir_canonical, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_poller_files(
    root: &std::path::Path,
    current: &std::path::Path,
    extensions: &[String],
    dir_canonical: &std::path::Path,
    files: &mut Vec<(String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    let depth = if current == root { 0usize } else { 1 };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() && depth == 0 {
            collect_poller_files(root, &path, extensions, dir_canonical, files);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if let Ok(canonical) = path.canonicalize()
            && !canonical.starts_with(dir_canonical)
        {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        if file_name == "docker-compose.yaml" || file_name == "docker-compose.yml" {
            continue;
        }

        let rel_name = path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or(&file_name)
            .to_string();

        let has_extension = file_name.contains('.');
        let matches = if has_extension {
            extensions
                .iter()
                .any(|ext| file_name.ends_with(ext.as_str()))
        } else {
            EXTENSIONLESS_ALLOWLIST.contains(&file_name.as_str())
        };

        if !matches {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            files.push((rel_name, content));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use nexus_link_core::signing::queue_message;

    const NODE: &str = "node-1";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[3u8; 32])
    }

    fn ctx<'a>(
        dir: &Path,
        require: bool,
        vk: Option<&'a VerifyingKey>,
        nonces: &'a NonceCache,
    ) -> CommandContext<'a> {
        CommandContext {
            node_id: NODE,
            compose_dir: dir.to_path_buf(),
            extra_extensions: &[],
            require_signatures: require,
            signing_keys: vk.map(std::slice::from_ref).unwrap_or(&[]),
            nonces,
            compose_writable: true,
        }
    }

    fn signed(kind: ComposeCommandType, payload: &str, nonce: &str) -> ComposeCommandItem {
        let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let msg = queue_message(NODE, "cmd-1", kind.as_str(), &ts, nonce, payload);
        ComposeCommandItem {
            id: "cmd-1".into(),
            command_type: kind,
            payload: Some(payload.into()),
            signature: Some(hex::encode(key().sign(msg.as_bytes()).to_bytes())),
            timestamp: Some(ts),
            nonce: Some(nonce.into()),
            ..Default::default()
        }
    }

    fn unsigned_put(content: &str) -> ComposeCommandItem {
        ComposeCommandItem {
            id: "cmd-1".into(),
            command_type: ComposeCommandType::PutFile,
            args: serde_json::json!({ "content": content }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn unsigned_put_file_is_rejected_and_not_written_when_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let result = handle_command(
            &unsigned_put("services: {}"),
            &ctx(dir.path(), true, Some(&vk), &nonces),
        )
        .await;
        assert_eq!(result.status, ComposeCommandStatus::Failed);
        assert!(result.error.unwrap().starts_with("signature: missing"));
        assert!(!dir.path().join("docker-compose.yaml").exists());
    }

    #[tokio::test]
    async fn tampered_put_file_is_rejected_and_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let mut cmd = signed(ComposeCommandType::PutFile, r#"{"content":"a: 1"}"#, "n1");
        cmd.payload = Some(r#"{"content":"evil: 1"}"#.into());
        // Rejected regardless of the enforcement flag
        for require in [true, false] {
            let result = handle_command(&cmd, &ctx(dir.path(), require, Some(&vk), &nonces)).await;
            assert_eq!(result.status, ComposeCommandStatus::Failed);
            assert_eq!(result.error.as_deref(), Some("signature: invalid"));
        }
        assert!(!dir.path().join("docker-compose.yaml").exists());
    }

    #[tokio::test]
    async fn enforced_without_key_rejects_activate() {
        let dir = tempfile::tempdir().unwrap();
        let nonces = NonceCache::new();
        let cmd = signed(ComposeCommandType::Activate, "{}", "n1");
        let result = handle_command(&cmd, &ctx(dir.path(), true, None, &nonces)).await;
        assert_eq!(result.status, ComposeCommandStatus::Failed);
        assert_eq!(
            result.error.as_deref(),
            Some("signature: no verifying key on node")
        );
    }

    #[tokio::test]
    async fn signed_put_file_is_written_when_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let cmd = signed(
            ComposeCommandType::PutFile,
            r#"{"content":"services:\n  app:\n    image: busybox\n"}"#,
            "n1",
        );
        let result = handle_command(&cmd, &ctx(dir.path(), true, Some(&vk), &nonces)).await;
        assert_eq!(
            result.status,
            ComposeCommandStatus::Completed,
            "{:?}",
            result.error
        );
        let written = std::fs::read_to_string(dir.path().join("docker-compose.yaml")).unwrap();
        assert_eq!(written, "services:\n  app:\n    image: busybox\n");

        // The same signed command cannot be replayed
        let replay = handle_command(&cmd, &ctx(dir.path(), true, Some(&vk), &nonces)).await;
        assert_eq!(replay.error.as_deref(), Some("signature: duplicate nonce"));
    }

    #[tokio::test]
    async fn unsigned_put_file_still_works_when_not_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let nonces = NonceCache::new();
        let result = handle_command(
            &unsigned_put("services: {}"),
            &ctx(dir.path(), false, None, &nonces),
        )
        .await;
        assert_eq!(result.status, ComposeCommandStatus::Completed);
        assert!(dir.path().join("docker-compose.yaml").exists());
    }

    #[tokio::test]
    async fn signed_validate_yaml_uses_payload_args() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let mut cmd = signed(
            ComposeCommandType::ValidateYaml,
            r#"{"content":"a: ["}"#,
            "n1",
        );
        cmd.args = serde_json::json!({ "content": "a: 1" });
        let result = handle_command(&cmd, &ctx(dir.path(), true, Some(&vk), &nonces)).await;
        assert_eq!(result.status, ComposeCommandStatus::Completed);
        assert_eq!(result.result.unwrap()["valid"], false);
    }

    #[tokio::test]
    async fn read_only_node_rejects_writes_but_serves_reads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docker-compose.yaml"), "services: {}\n").unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let mut ro = ctx(dir.path(), true, Some(&vk), &nonces);
        ro.compose_writable = false;

        let put = signed(ComposeCommandType::PutFile, r#"{"content":"a: 1"}"#, "n1");
        let result = handle_command(&put, &ro).await;
        assert_eq!(result.status, ComposeCommandStatus::Failed);
        assert_eq!(result.error.as_deref(), Some(COMPOSE_READ_ONLY_ERROR));
        let content = std::fs::read_to_string(dir.path().join("docker-compose.yaml")).unwrap();
        assert_eq!(content, "services: {}\n", "file must stay unchanged");

        let activate = signed(ComposeCommandType::Activate, "{}", "n2");
        let result = handle_command(&activate, &ro).await;
        assert_eq!(result.error.as_deref(), Some(COMPOSE_READ_ONLY_ERROR));

        let get = signed(ComposeCommandType::GetFile, "{}", "n3");
        let result = handle_command(&get, &ro).await;
        assert_eq!(result.status, ComposeCommandStatus::Completed);

        let validate = signed(
            ComposeCommandType::ValidateYaml,
            r#"{"content":"a: 1"}"#,
            "n4",
        );
        let result = handle_command(&validate, &ro).await;
        assert_eq!(result.status, ComposeCommandStatus::Completed);
    }

    #[tokio::test]
    async fn read_only_node_reports_forged_write_as_signature_error() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let mut ro = ctx(dir.path(), true, Some(&vk), &nonces);
        ro.compose_writable = false;
        let mut put = signed(ComposeCommandType::PutFile, r#"{"content":"a: 1"}"#, "n1");
        put.payload = Some(r#"{"content":"evil: 1"}"#.into());
        let result = handle_command(&put, &ro).await;
        assert_eq!(result.error.as_deref(), Some("signature: invalid"));
    }

    #[tokio::test]
    async fn signed_unknown_type_is_reported_unsupported_and_not_executed() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let cmd = signed(ComposeCommandType::Unknown("compose_ps".into()), "{}", "n1");
        let result = handle_command(&cmd, &ctx(dir.path(), true, Some(&vk), &nonces)).await;
        assert_eq!(result.status, ComposeCommandStatus::Failed);
        assert_eq!(
            result.error.as_deref(),
            Some("unsupported command type: compose_ps")
        );
        assert!(result.result.is_none());
    }

    #[tokio::test]
    async fn forged_unknown_type_reports_the_signature_error_first() {
        let dir = tempfile::tempdir().unwrap();
        let vk = key().verifying_key();
        let nonces = NonceCache::new();
        let mut cmd = signed(ComposeCommandType::Unknown("read_file".into()), "{}", "n1");
        cmd.payload = Some(r#"{"path":"/etc/shadow"}"#.into());
        let result = handle_command(&cmd, &ctx(dir.path(), true, Some(&vk), &nonces)).await;
        assert_eq!(result.error.as_deref(), Some("signature: invalid"));
    }

    #[test]
    fn mixed_pending_list_parses_item_by_item() {
        let list: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
                {"id":"a","type":"get_file"},
                {"id":"b","type":"compose_ps","payload":"{}"},
                {"id":"c","type":42},
                {"type":"activate"},
                {"id":"","type":"activate"},
                {"id":"d","type":"validate_yaml","args":{"content":"a: 1"}}
            ]"#,
        )
        .unwrap();
        let parsed: Vec<ParsedItem> = list.into_iter().map(parse_item).collect();

        assert!(matches!(&parsed[0], ParsedItem::Command(c)
            if c.id == "a" && c.command_type == ComposeCommandType::GetFile));
        assert!(matches!(&parsed[1], ParsedItem::Command(c)
            if c.id == "b" && c.command_type == ComposeCommandType::Unknown("compose_ps".into())));
        assert!(matches!(&parsed[2], ParsedItem::Malformed { id, error }
            if id == "c" && error.starts_with("malformed command:")));
        assert!(matches!(parsed[3], ParsedItem::NoId));
        assert!(matches!(parsed[4], ParsedItem::NoId));
        assert!(matches!(&parsed[5], ParsedItem::Command(c)
            if c.id == "d" && c.command_type == ComposeCommandType::ValidateYaml));
    }

    #[test]
    fn unknown_type_keeps_its_wire_name() {
        let t: ComposeCommandType = serde_json::from_str(r#""read_file""#).unwrap();
        assert_eq!(t, ComposeCommandType::Unknown("read_file".into()));
        assert_eq!(t.as_str(), "read_file");
        assert_eq!(serde_json::to_string(&t).unwrap(), r#""read_file""#);
        // Known names never fall into Unknown
        let k: ComposeCommandType = serde_json::from_str(r#""put_file""#).unwrap();
        assert_eq!(k, ComposeCommandType::PutFile);
    }
}
