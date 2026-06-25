use nexus_link_core::config::{self, Config, dirs_home};
use std::io::{self, Write};
use tracing::info;

/// Hard-reset the nexus-link installation on this device.
///
/// Removes all local credentials, keys, and configuration, and stops all
/// nexus-link systemd services. Intended for use after a device has been
/// deleted in the Nexus dashboard and needs a clean slate for re-registration.
///
/// Unlike `unregister`, reset:
///   - Does NOT send any heartbeat to the backend (device may already be gone)
///   - Stops AND disables both nexus-link-agent and nexus-link-service
///   - Removes all files in ~/.nexus-link/ (config, keys, state)
///   - Never touches Docker containers or compose files
pub async fn execute(force: bool) -> anyhow::Result<()> {
    let home = dirs_home();
    let config_path = config::default_config_path();

    // Describe what will be removed
    let node_info = if config_path.exists() {
        match Config::load() {
            Ok(c) => format!("node '{}' (ID: {})", c.node.name, c.node.node_id),
            Err(_) => "node (config unreadable)".to_string(),
        }
    } else {
        "node (no config found)".to_string()
    };

    if !force {
        println!("This will RESET all nexus-link state for {}.", node_info);
        println!();
        println!("  The following will be removed:");
        println!("    {}  (entire directory)", home.display());
        println!("  The following services will be stopped and disabled:");
        println!("    nexus-link-agent");
        println!("    nexus-link-service");
        println!();
        println!("  Docker containers and compose files are NOT affected.");
        println!();
        print!("Proceed? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!("Resetting nexus-link on {}...", node_info);
    println!();

    // 1. Stop and disable all nexus-link systemd services
    stop_and_disable_services();

    // 2. Remove the entire ~/.nexus-link/ directory
    remove_home_dir(&home, force)?;

    println!();
    println!("Reset complete.");
    println!("  All credentials and config removed.");
    println!("  Run 'nexus-link register' to re-register this device.");

    Ok(())
}

/// Stop and disable nexus-link-agent and nexus-link-service via systemd.
/// Tries user-mode first, falls back to system-mode, then pkill.
/// Never fails — best effort only.
fn stop_and_disable_services() {
    for service in &["nexus-link-agent", "nexus-link-service"] {
        stop_service(service);
    }
}

fn stop_service(service: &str) {
    // Try systemctl --user first
    let user_stop = std::process::Command::new("systemctl")
        .args(["--user", "stop", service])
        .output();

    if let Ok(o) = user_stop {
        if o.status.success() {
            println!("  Stopped (user service): {}", service);
            disable_service(service, true);
            return;
        }
    }

    // Try system-wide
    let system_stop = std::process::Command::new("systemctl")
        .args(["stop", service])
        .output();

    if let Ok(o) = system_stop {
        if o.status.success() {
            println!("  Stopped (system service): {}", service);
            disable_service(service, false);
            return;
        }
    }

    // Fall back to pkill
    let pkill = std::process::Command::new("pkill")
        .args(["-f", service])
        .output();

    match pkill {
        Ok(o) if o.status.success() => println!("  Killed process: {}", service),
        _ => println!("  Not running: {}", service),
    }
}

fn disable_service(service: &str, user: bool) {
    let mut args = vec![];
    if user {
        args.push("--user");
    }
    args.extend_from_slice(&["disable", service]);

    let result = std::process::Command::new("systemctl").args(&args).output();

    if let Ok(o) = result {
        if o.status.success() {
            info!("Disabled service: {}", service);
        }
    }
}

/// Remove the entire ~/.nexus-link/ directory.
fn remove_home_dir(home: &std::path::Path, force: bool) -> anyhow::Result<()> {
    if !home.exists() {
        println!(
            "  Config directory not found: {} (nothing to remove)",
            home.display()
        );
        return Ok(());
    }

    match std::fs::remove_dir_all(home) {
        Ok(()) => {
            println!("  Removed: {}", home.display());
            Ok(())
        }
        Err(e) if force => {
            println!("  Warning: could not remove {} — {}", home.display(), e);
            Ok(())
        }
        Err(e) => anyhow::bail!("Failed to remove {}: {}", home.display(), e),
    }
}
