use nexus_link_core::config::Config;
use tracing::info;

pub async fn execute(token: String) -> anyhow::Result<()> {
    // Validate token format
    if !nexus_link_core::token::validate_token_format(&token) {
        anyhow::bail!("Invalid token format. Expected: nxs_node_<...>");
    }

    // Load current config
    let mut config = Config::load()?;
    let old_token = config.node.token.clone();

    if old_token == token {
        println!("Token is already the same. Nothing to do.");
        return Ok(());
    }

    println!("Refreshing token for node '{}'...", config.node.name);

    // Replace with new token — keep both locations in sync
    config.node.token = token.clone();
    if let Some(ref mut t) = config.api.tokens.telemetry {
        t.token = token.clone();
    } else {
        config.api.tokens.telemetry = Some(nexus_link_core::config::TokenEntry {
            token: token.clone(),
            scope: "read".to_string(),
        });
    }

    // Attempt heartbeat with the new token
    info!("Sending heartbeat with new token...");
    let heartbeat_ok = send_heartbeat(&config).await;

    if heartbeat_ok {
        // New token accepted -- persist config
        config.save()?;
        println!("  Token refreshed successfully.");
        println!("  Old token will be invalidated after the grace period (24h).");

        // Restart agent if running so it picks up the new token
        restart_agent();
    } else {
        // New token rejected -- revert
        println!("  New token rejected by backend. Old token restored.");
        println!("  Verify the token was copied correctly from the dashboard.");
        // Config was not saved, so old token is still on disk
        anyhow::bail!("Token refresh failed. No changes made.");
    }

    Ok(())
}

/// Send a heartbeat to verify the token is accepted by the backend
async fn send_heartbeat(config: &Config) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let url = format!(
        "{}/api/nodes/{}/heartbeat",
        config.api.base_url, config.node.node_id
    );

    let body = serde_json::json!({
        "status": "online",
        "reason": "token_refresh"
    });

    match client
        .post(&url)
        .bearer_auth(&config.node.token)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            info!("Heartbeat accepted with new token");
            println!("  Heartbeat: accepted (200)");
            true
        }
        Ok(resp) => {
            let status = resp.status();
            info!("Heartbeat rejected: {}", status);
            println!("  Heartbeat: rejected ({})", status);
            false
        }
        Err(e) => {
            info!("Heartbeat failed: {}", e);
            println!("  Heartbeat: network error ({})", e);
            false
        }
    }
}

/// Restart the agent service so it picks up the new token
fn restart_agent() {
    // Try systemd user mode first, then system mode
    let user_restart = std::process::Command::new("systemctl")
        .args(["--user", "restart", "nexus-link-agent"])
        .output();

    if let Ok(o) = user_restart
        && o.status.success()
    {
        println!("  Agent restarted (user service)");
        return;
    }

    let system_restart = std::process::Command::new("systemctl")
        .args(["restart", "nexus-link-agent"])
        .output();

    if let Ok(o) = system_restart
        && o.status.success()
    {
        println!("  Agent restarted (system service)");
        return;
    }

    // Try sudo
    let sudo_restart = std::process::Command::new("sudo")
        .args(["-n", "systemctl", "restart", "nexus-link-agent"])
        .output();

    if let Ok(o) = sudo_restart
        && o.status.success()
    {
        println!("  Agent restarted (sudo)");
        return;
    }

    println!("  Agent not running as a service. Restart manually if needed.");
}
