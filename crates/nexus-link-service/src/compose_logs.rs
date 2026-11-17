//! Log snapshots of the compose stack without `docker compose`.
//!
//! `docker compose logs` loads the project first: it parses the compose file
//! and reads `.env` for variable interpolation, even though logs need
//! neither. On hardened hosts `.env` holds secrets the service user cannot
//! (and should not) read, so the call fails with `permission denied`.
//!
//! Instead, the containers of the stack are found through the label compose
//! puts on every container (`com.docker.compose.project.working_dir`), and
//! their logs are read with `docker logs`. Nothing in the compose directory
//! is opened.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Compose label holding the absolute project directory.
const WORKING_DIR_LABEL: &str = "com.docker.compose.project.working_dir";
/// Compose label holding the service name.
const SERVICE_LABEL: &str = "com.docker.compose.service";

/// Upper bound for lines returned to the hub; the newest lines are kept.
pub const MAX_LINES: usize = 500;
/// Maximum length of an error message reported to the hub (SEC-008).
const MAX_ERROR_LEN: usize = 2048;

/// A container of the compose stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackContainer {
    pub service: String,
    pub name: String,
}

/// Successful snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub lines: Vec<String>,
    pub truncated: bool,
    pub containers: usize,
}

/// Snapshot request.
pub struct SnapshotRequest<'a> {
    pub compose_dir: &'a Path,
    pub service: Option<&'a str>,
    pub tail: u64,
    /// Passed to `docker logs --since`, e.g. `5m`.
    pub since: Option<&'a str>,
    pub timeout: Duration,
}

/// Collect a log snapshot. `Err` carries a message for the hub.
pub async fn snapshot(req: &SnapshotRequest<'_>) -> Result<Snapshot, String> {
    match timeout(req.timeout, collect(req)).await {
        Ok(result) => result,
        Err(_) => Err("Timeout collecting log snapshot".to_string()),
    }
}

async fn collect(req: &SnapshotRequest<'_>) -> Result<Snapshot, String> {
    let mut found = list_containers(req.compose_dir).await?;
    // Compose records the directory it was started from; if `compose.dir`
    // goes through a symlink, the label may hold the resolved path instead.
    if found.is_empty()
        && let Ok(canonical) = std::fs::canonicalize(req.compose_dir)
        && canonical != req.compose_dir
    {
        found = list_containers(&canonical).await?;
    }

    let containers: Vec<StackContainer> = found
        .into_iter()
        .filter(|c| req.service.is_none_or(|s| c.service == s))
        .collect();

    let tail = req.tail.to_string();
    let mut per_container = Vec::with_capacity(containers.len());
    for c in &containers {
        let mut args = vec!["logs", "--timestamps", "--tail", &tail];
        if let Some(since) = req.since {
            args.extend(["--since", since]);
        }
        args.push(&c.name);
        let out = run_docker_combined(&args).await?;
        per_container.push((c.service.clone(), out));
    }

    let (lines, truncated) = merge_lines(&per_container, MAX_LINES);
    Ok(Snapshot {
        lines,
        truncated,
        containers: containers.len(),
    })
}

/// Containers (running or not) whose compose working dir is `dir`.
async fn list_containers(dir: &Path) -> Result<Vec<StackContainer>, String> {
    let label = format!("label={WORKING_DIR_LABEL}={}", dir.display());
    let format = format!("{{{{.Label \"{SERVICE_LABEL}\"}}}}\t{{{{.Names}}}}");
    let ps = run_docker(&["ps", "-a", "--filter", &label, "--format", &format]).await?;
    Ok(parse_ps(&ps))
}

/// Parse `docker ps` output of the form `<service>\t<name>` per line.
pub fn parse_ps(output: &str) -> Vec<StackContainer> {
    output
        .lines()
        .filter_map(|line| {
            let (service, name) = line.split_once('\t')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(StackContainer {
                service: service.trim().to_string(),
                name: name.to_string(),
            })
        })
        .collect()
}

/// Merge per-container `docker logs --timestamps` output into one list,
/// ordered by timestamp, each line prefixed with its service. Keeps the
/// newest `max` lines; returns whether lines were dropped.
pub fn merge_lines(per_container: &[(String, String)], max: usize) -> (Vec<String>, bool) {
    let width = per_container
        .iter()
        .map(|(s, _)| s.len())
        .max()
        .unwrap_or(0);

    let mut entries: Vec<(&str, String)> = per_container
        .iter()
        .flat_map(|(service, out)| {
            out.lines().filter(|l| !l.is_empty()).map(move |line| {
                // RFC 3339 timestamps from docker logs sort lexically (UTC).
                let ts = line.split_once(' ').map_or("", |(ts, _)| ts);
                (ts, format!("{service:<width$} | {line}"))
            })
        })
        .collect();
    // Stable sort keeps the per-container order for equal timestamps.
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let truncated = entries.len() > max;
    let skip = entries.len().saturating_sub(max);
    let lines = entries.into_iter().skip(skip).map(|(_, l)| l).collect();
    (lines, truncated)
}

/// Run `docker` and return stdout; a non-zero exit is an error with stderr.
async fn run_docker(args: &[&str]) -> Result<String, String> {
    let output = docker(args).await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(failure(args, &output))
    }
}

/// Run `docker logs`: the container's stdout and stderr are both log output.
async fn run_docker_combined(args: &[&str]) -> Result<String, String> {
    let output = docker(args).await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr))
    } else {
        Err(failure(args, &output))
    }
}

async fn docker(args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Failed to run docker {}: {e}", args[0]))
}

fn failure(args: &[&str], output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output
        .status
        .code()
        .map_or_else(|| "signal".to_string(), |c| c.to_string());
    let msg = format!("docker {} failed (exit {code}): {}", args[0], stderr.trim());
    truncate_str(&msg, MAX_ERROR_LEN)
}

/// Truncate at a char boundary.
pub fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let end = s.floor_char_boundary(max);
    format!("{}... [truncated]", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_reads_service_and_name() {
        let out = "vllm\tdgx-llm-vllm-1\nlitellm\tdgx-llm-litellm-1\n\nbroken line\n\t\n";
        assert_eq!(
            parse_ps(out),
            vec![
                StackContainer {
                    service: "vllm".into(),
                    name: "dgx-llm-vllm-1".into()
                },
                StackContainer {
                    service: "litellm".into(),
                    name: "dgx-llm-litellm-1".into()
                },
            ]
        );
    }

    #[test]
    fn merge_orders_by_timestamp_and_prefixes_service() {
        let per = vec![
            (
                "vllm".to_string(),
                "2026-11-17T05:41:02.000000001Z b\n2026-11-17T05:41:04.000000000Z d\n".to_string(),
            ),
            (
                "db".to_string(),
                "2026-11-17T05:41:01.000000000Z a\n2026-11-17T05:41:03.000000000Z c\n".to_string(),
            ),
        ];
        let (lines, truncated) = merge_lines(&per, 10);
        assert!(!truncated);
        assert_eq!(
            lines,
            vec![
                "db   | 2026-11-17T05:41:01.000000000Z a",
                "vllm | 2026-11-17T05:41:02.000000001Z b",
                "db   | 2026-11-17T05:41:03.000000000Z c",
                "vllm | 2026-11-17T05:41:04.000000000Z d",
            ]
        );
    }

    #[test]
    fn merge_keeps_newest_lines_when_capped() {
        let out: String = (0..5)
            .map(|i| format!("2026-11-17T05:41:0{i}Z line{i}\n"))
            .collect();
        let (lines, truncated) = merge_lines(&[("s".into(), out)], 2);
        assert!(truncated);
        assert_eq!(
            lines,
            vec![
                "s | 2026-11-17T05:41:03Z line3",
                "s | 2026-11-17T05:41:04Z line4"
            ]
        );
    }

    #[test]
    fn merge_of_nothing_is_empty() {
        assert_eq!(merge_lines(&[], 10), (vec![], false));
    }

    /// Runs against a live compose stack:
    /// `NEXUS_LINK_TEST_COMPOSE_DIR=<dir> cargo test -p nexus-link-service -- --ignored`
    #[tokio::test]
    #[ignore = "needs docker and a running compose stack"]
    async fn live_snapshot_without_reading_env() {
        let dir =
            std::env::var("NEXUS_LINK_TEST_COMPOSE_DIR").expect("set NEXUS_LINK_TEST_COMPOSE_DIR");
        let req = SnapshotRequest {
            compose_dir: Path::new(&dir),
            service: None,
            tail: 50,
            since: None,
            timeout: Duration::from_secs(15),
        };
        let snap = snapshot(&req).await.expect("snapshot");
        println!("{snap:#?}");
        assert!(snap.containers > 0);
        assert!(!snap.lines.is_empty());

        let only_worker = SnapshotRequest {
            service: Some("worker"),
            ..req
        };
        let snap = snapshot(&only_worker).await.expect("snapshot");
        assert_eq!(snap.containers, 1);
        assert!(snap.lines.iter().all(|l| l.starts_with("worker")));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "ä".repeat(10); // 2 bytes per char
        let t = truncate_str(&s, 5);
        assert!(t.starts_with("ää"));
        assert!(t.ends_with("... [truncated]"));
        assert_eq!(truncate_str("short", 10), "short");
    }
}
