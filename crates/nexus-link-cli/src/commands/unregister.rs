use nexus_link_core::config::{self, Config};
use std::io::{self, Write};
use tracing::info;

pub async fn execute(force: bool) -> anyhow::Result<()> {
    let config_path = config::default_config_path();

    // Check if config exists
    if !config_path.exists() {
        if force {
            println!("No config found. Nothing to unregister.");
            return Ok(());
        } else {
            anyhow::bail!(
                "No config found at {}. Node is not registered.",
                config_path.display()
            );
        }
    }

    // Load config for the offline heartbeat
    let config = Config::load_from(config_path.clone())?;

    // Confirmation prompt (unless --force)
    if !force {
        print!(
            "Unregister node '{}' (ID: {})? This will remove local credentials. [y/N] ",
            config.node.name, config.node.node_id
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!("Unregistering node '{}'...", config.node.name);

    // 1. Send offline heartbeat (best-effort)
    send_offline_heartbeat(&config).await;

    // 2. Stop the agent if running
    stop_agent().await;

    // 3. Remove local config
    match std::fs::remove_file(&config_path) {
        Ok(()) => {
            info!("Removed config: {}", config_path.display());
        }
        Err(e) if force => {
            info!("Could not remove config (ignored): {}", e);
        }
        Err(e) => {
            anyhow::bail!("Failed to remove config: {}", e);
        }
    }

    // Also remove the config directory if empty
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::remove_dir(parent); // only succeeds if empty
    }

    println!();
    println!("Node unregistered. Local config removed.");
    println!("  If the agent was running as a systemd service, also run:");
    println!("    sudo systemctl stop nexus-link-agent");
    println!("    sudo systemctl disable nexus-link-agent");

    Ok(())
}

/// Send a final offline heartbeat to the backend (best-effort, never fails the command)
async fn send_offline_heartbeat(config: &Config) {
    info!("Sending offline heartbeat...");

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let url = format!(
        "{}/api/nodes/{}/heartbeat",
        config.api.base_url, config.node.node_id
    );

    let body = serde_json::json!({
        "status": "offline",
        "reason": "unregister"
    });

    match client
        .post(&url)
        .bearer_auth(&config.node.token)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            info!("Offline heartbeat sent successfully");
            println!("  Notified backend: node offline");
        }
        Ok(resp) => {
            info!("Offline heartbeat returned {}", resp.status());
            println!("  Backend notification skipped ({})", resp.status());
        }
        Err(e) => {
            info!("Offline heartbeat failed: {}", e);
            println!("  Backend notification skipped (unreachable)");
        }
    }
}

/// Stop the running agent process (best-effort)
async fn stop_agent() {
    info!("Attempting to stop agent...");

    // Try systemctl first (common on Linux nodes)
    let systemctl = std::process::Command::new("systemctl")
        .args(["stop", "nexus-link-agent"])
        .output();

    match systemctl {
        Ok(output) if output.status.success() => {
            println!("  Agent service stopped");
            return;
        }
        _ => {}
    }

    // Try pkill as fallback
    let pkill = std::process::Command::new("pkill")
        .args(["-f", "nexus-link-agent"])
        .output();

    match pkill {
        Ok(output) if output.status.success() => {
            println!("  Agent process stopped");
        }
        _ => {
            println!("  No running agent found (or already stopped)");
        }
    }
}
