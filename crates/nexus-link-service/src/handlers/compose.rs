use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use tokio_stream::wrappers::LinesStream;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use crate::state::SharedState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ComposeFileEntry {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct GetComposeFileResponse {
    pub path: String,
    pub content: String,
    pub files: Vec<ComposeFileEntry>,
}

#[derive(Debug, Deserialize)]
pub struct PutComposeFileRequest {
    pub content: String,
    #[serde(default = "default_commit_message")]
    pub message: String,
}

fn default_commit_message() -> String {
    "nexus-link: update compose config".to_string()
}

#[derive(Debug, Serialize)]
pub struct PutComposeFileResponse {
    pub path: String,
    pub committed: bool,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActivateComposeResponse {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub service: Option<String>,
    #[serde(default = "default_log_tail")]
    pub tail: u32,
}

fn default_log_tail() -> u32 {
    200
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the compose file in the given directory (tries .yaml then .yml).
fn find_compose_file(dir: &Path) -> Option<PathBuf> {
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

/// Read extra config files from the compose directory that match the allowed extensions.
fn read_extra_files(dir: &Path, extensions: &[String]) -> Vec<ComposeFileEntry> {
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
        // Skip the compose file itself
        if name == "docker-compose.yaml" || name == "docker-compose.yml" {
            continue;
        }
        let matches = extensions
            .iter()
            .any(|ext| name.ends_with(ext.as_str()) || name.starts_with(ext.trim_start_matches('.')));
        if !matches {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => files.push(ComposeFileEntry { name, content }),
            Err(e) => warn!("Could not read extra file {}: {}", path.display(), e),
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// Check if a directory is a git repository.
fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Run a git add + commit for the compose file. Returns the short commit SHA.
async fn git_commit(dir: &Path, message: &str) -> Option<String> {
    let add = Command::new("git")
        .args(["add", "docker-compose.yaml"])
        .current_dir(dir)
        .output()
        .await;

    if let Err(e) = add {
        warn!("git add failed: {}", e);
        return None;
    }

    let commit = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .await;

    match commit {
        Ok(out) if out.status.success() => {
            // Get the short SHA of the new commit
            if let Ok(sha_out) = Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(dir)
                .output()
                .await
            {
                let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();
                if !sha.is_empty() { Some(sha) } else { None }
            } else {
                None
            }
        }
        Ok(out) => {
            // Nothing to commit (already up-to-date) is not an error
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("nothing to commit") {
                info!("git commit: nothing to commit");
            } else {
                warn!("git commit returned non-zero: {}", stderr);
            }
            None
        }
        Err(e) => {
            warn!("git commit failed: {}", e);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// GET /api/compose/file
// ---------------------------------------------------------------------------

pub async fn get_compose_file(
    State(state): State<SharedState>,
) -> Result<Json<GetComposeFileResponse>, (StatusCode, String)> {
    let compose_dir = &state.config.compose.dir;

    let compose_path = find_compose_file(compose_dir).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!(
                "No docker-compose.yaml found in {}",
                compose_dir.display()
            ),
        )
    })?;

    let content = std::fs::read_to_string(&compose_path).map_err(|e| {
        error!("Failed to read compose file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read compose file: {}", e),
        )
    })?;

    let extra_files = read_extra_files(compose_dir, &state.config.compose.extra_extensions);

    info!(
        path = %compose_path.display(),
        extra_files = extra_files.len(),
        "Compose file read"
    );

    Ok(Json(GetComposeFileResponse {
        path: compose_path.to_string_lossy().to_string(),
        content,
        files: extra_files,
    }))
}

// ---------------------------------------------------------------------------
// PUT /api/compose/file
// ---------------------------------------------------------------------------

pub async fn put_compose_file(
    State(state): State<SharedState>,
    Json(body): Json<PutComposeFileRequest>,
) -> Result<Json<PutComposeFileResponse>, (StatusCode, String)> {
    // Validate YAML syntax before writing
    serde_yaml::from_str::<serde_yaml::Value>(&body.content).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid YAML: {}", e),
        )
    })?;

    let compose_dir = &state.config.compose.dir;

    // Ensure directory exists
    std::fs::create_dir_all(compose_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Cannot create compose directory: {}", e),
        )
    })?;

    let compose_path = compose_dir.join("docker-compose.yaml");
    let tmp_path = compose_dir.join("docker-compose.yaml.tmp");

    // Atomic write: write to .tmp, then rename
    std::fs::write(&tmp_path, &body.content).map_err(|e| {
        error!("Failed to write tmp compose file: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write file: {}", e),
        )
    })?;

    std::fs::rename(&tmp_path, &compose_path).map_err(|e| {
        error!("Failed to rename tmp compose file: {}", e);
        let _ = std::fs::remove_file(&tmp_path);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to commit file write: {}", e),
        )
    })?;

    info!(path = %compose_path.display(), "Compose file written");

    // Git commit if the directory is a git repo
    let (committed, commit_sha) = if is_git_repo(compose_dir) {
        let sha = git_commit(compose_dir, &body.message).await;
        (true, sha)
    } else {
        (false, None)
    };

    Ok(Json(PutComposeFileResponse {
        path: compose_path.to_string_lossy().to_string(),
        committed,
        commit_sha,
    }))
}

// ---------------------------------------------------------------------------
// POST /api/compose/activate
// ---------------------------------------------------------------------------

pub async fn activate_compose(
    State(state): State<SharedState>,
) -> Result<Json<ActivateComposeResponse>, (StatusCode, String)> {
    let compose_dir = state.config.compose.dir.clone();

    info!(dir = %compose_dir.display(), "Running docker compose up -d");

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
        Err(_elapsed) => {
            error!("docker compose up -d timed out after 120s");
            Ok(Json(ActivateComposeResponse {
                success: false,
                exit_code: -1,
                stdout: String::new(),
                stderr: "Timeout: docker compose up -d did not complete within 120 seconds"
                    .to_string(),
            }))
        }
        Ok(Err(e)) => {
            error!("Failed to spawn docker compose: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to spawn docker compose: {}", e),
            ))
        }
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let success = output.status.success();

            if success {
                info!("docker compose up -d succeeded");
            } else {
                warn!(exit_code, "docker compose up -d failed: {}", stderr);
            }

            Ok(Json(ActivateComposeResponse {
                success,
                exit_code,
                stdout,
                stderr,
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// GET /api/compose/logs  (Server-Sent Events)
// ---------------------------------------------------------------------------

pub async fn stream_compose_logs(
    State(state): State<SharedState>,
    Query(params): Query<LogsQuery>,
) -> impl IntoResponse {
    let compose_dir = state.config.compose.dir.clone();
    let tail = params.tail.to_string();

    let mut args = vec![
        "compose".to_string(),
        "logs".to_string(),
        "--follow".to_string(),
        "--no-color".to_string(),
        "--tail".to_string(),
        tail,
    ];

    if let Some(service) = &params.service {
        args.push(service.clone());
    }

    info!(
        dir = %compose_dir.display(),
        service = ?params.service,
        "Starting compose log stream"
    );

    let stream = async_stream::stream! {
        let child = Command::new("docker")
            .args(&args)
            .current_dir(&compose_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to spawn docker compose logs: {}", e);
                yield Ok::<Event, std::convert::Infallible>(
                    Event::default().data(format!("error: {}", e))
                );
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                yield Ok(Event::default().data("error: could not capture stdout"));
                return;
            }
        };

        let reader = BufReader::new(stdout);
        let mut lines = LinesStream::new(reader.lines());

        while let Some(line) = lines.next().await {
            match line {
                Ok(l) => yield Ok(Event::default().data(l)),
                Err(e) => {
                    warn!("Log stream read error: {}", e);
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_compose_file_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("docker-compose.yaml");
        std::fs::write(&path, "version: '3'\n").unwrap();
        assert_eq!(find_compose_file(dir.path()), Some(path));
    }

    #[test]
    fn test_find_compose_file_yml_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("docker-compose.yml");
        std::fs::write(&path, "version: '3'\n").unwrap();
        assert_eq!(find_compose_file(dir.path()), Some(path));
    }

    #[test]
    fn test_find_compose_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_compose_file(dir.path()), None);
    }

    #[test]
    fn test_yaml_validation_valid() {
        let yaml = "services:\n  vllm:\n    image: vllm/vllm-openai\n";
        assert!(serde_yaml::from_str::<serde_yaml::Value>(yaml).is_ok());
    }

    #[test]
    fn test_yaml_validation_invalid() {
        let bad = "services:\n  vllm\n    image: vllm\n"; // missing colon after 'vllm'
        assert!(serde_yaml::from_str::<serde_yaml::Value>(bad).is_err());
    }

    #[test]
    fn test_read_extra_files_filters_correctly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "FOO=bar\n").unwrap();
        std::fs::write(dir.path().join("docker-compose.yaml"), "version: '3'\n").unwrap();
        std::fs::write(dir.path().join("unrelated.rs"), "fn main() {}\n").unwrap();

        let extensions = vec![".env".to_string(), ".conf".to_string()];
        let files = read_extra_files(dir.path(), &extensions);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, ".env");
    }

    #[test]
    fn test_is_git_repo_false() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
    }

    #[test]
    fn test_is_git_repo_true() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_git_repo(dir.path()));
    }
}
