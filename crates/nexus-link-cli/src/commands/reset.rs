use nexus_link_core::config::{self, Config, SERVICE_USER, SYSTEM_CONFIG_DIR, dirs_home};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use tracing::info;

/// Hard-reset the nexus-link installation on this device.
///
/// Removes all local credentials, keys, and configuration, stops all
/// nexus-link systemd services, removes unit files, the system user,
/// and installed binaries. Intended for use after a device has been
/// deleted in the Nexus dashboard and needs a clean slate for re-registration.
///
/// Unlike `unregister`, reset:
///   - Does NOT send any heartbeat to the backend (device may already be gone)
///   - Stops AND disables both nexus-link-agent and nexus-link-service
///   - Removes systemd unit files and reloads daemon
///   - Removes all files in ~/.nexus-link/ AND /var/lib/nexus-link/
///   - Removes installed binaries from /usr/local/bin/
///   - Removes the nexus-link system user
///   - Never touches Docker containers or compose files
pub async fn execute(force: bool) -> anyhow::Result<()> {
    let home = dirs_home();
    let config_path = config::default_config_path();
    let system_dir = Path::new(SYSTEM_CONFIG_DIR);

    // Describe what will be removed
    let node_info = if config_path.exists() {
        match Config::load() {
            Ok(c) => format!("node '{}' (ID: {})", c.node.name, c.node.node_id),
            Err(_) => "node (config unreadable)".to_string(),
        }
    } else if system_dir.join("config.toml").exists() {
        "node (system config)".to_string()
    } else {
        "node (no config found)".to_string()
    };

    if !force {
        println!("This will RESET all nexus-link state for {}.", node_info);
        println!();
        println!("  The following will be removed:");
        println!("    {}  (legacy user directory)", home.display());
        println!("    {}  (system directory)", SYSTEM_CONFIG_DIR);
        println!("    /usr/local/bin/nexus-link*  (installed binaries)");
        println!("    /etc/systemd/system/nexus-link-*.service  (unit files)");
        println!("    System user '{}'", SERVICE_USER);
        println!();
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

    // 2. Remove systemd unit files
    remove_systemd_units();

    // 3. Remove the legacy ~/.nexus-link/ directory
    remove_dir_if_exists(&home, "legacy config dir");

    // 4. Remove the system /var/lib/nexus-link/ directory
    remove_system_dir(system_dir);

    // 5. Remove installed binaries from /usr/local/bin/
    remove_installed_binaries();

    // 6. Remove the system user
    remove_system_user();

    println!();
    println!("Reset complete.");
    println!("  All credentials, config, services, and binaries removed.");
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
    let user_stop = Command::new("systemctl")
        .args(["--user", "stop", service])
        .output();

    if let Ok(o) = user_stop
        && o.status.success()
    {
        println!("  Stopped (user service): {}", service);
        disable_service(service, true);
        return;
    }

    // Try system-wide (with sudo)
    let system_stop = Command::new("sudo")
        .args(["systemctl", "stop", service])
        .output();

    if let Ok(o) = system_stop
        && o.status.success()
    {
        println!("  Stopped (system service): {}", service);
        disable_service(service, false);
        return;
    }

    // Fall back to pkill
    let pkill = Command::new("pkill").args(["-f", service]).output();

    match pkill {
        Ok(o) if o.status.success() => println!("  Killed process: {}", service),
        _ => println!("  Not running: {}", service),
    }
}

fn disable_service(service: &str, user: bool) {
    let result = if user {
        Command::new("systemctl")
            .args(["--user", "disable", service])
            .output()
    } else {
        Command::new("sudo")
            .args(["systemctl", "disable", service])
            .output()
    };

    if let Ok(o) = result
        && o.status.success()
    {
        info!("Disabled service: {}", service);
    }
}

fn remove_systemd_units() {
    let units = &[
        "/etc/systemd/system/nexus-link-agent.service",
        "/etc/systemd/system/nexus-link-service.service",
    ];

    for unit in units {
        if Path::new(unit).exists() {
            let result = Command::new("sudo").args(["rm", "-f", unit]).output();
            match result {
                Ok(o) if o.status.success() => println!("  Removed: {}", unit),
                _ => println!("  Warning: could not remove {}", unit),
            }
        }
    }

    // Reload systemd daemon
    let _ = Command::new("sudo")
        .args(["systemctl", "daemon-reload"])
        .output();
    println!("  Systemd daemon reloaded");
}

fn remove_dir_if_exists(dir: &Path, label: &str) {
    if !dir.exists() {
        return;
    }

    match std::fs::remove_dir_all(dir) {
        Ok(()) => println!("  Removed: {} ({})", dir.display(), label),
        Err(e) => println!("  Warning: could not remove {} — {}", dir.display(), e),
    }
}

fn remove_system_dir(dir: &Path) {
    if !dir.exists() {
        return;
    }

    let result = Command::new("sudo")
        .args(["rm", "-rf", &dir.to_string_lossy()])
        .output();

    match result {
        Ok(o) if o.status.success() => println!("  Removed: {} (system config dir)", dir.display()),
        _ => println!("  Warning: could not remove {}", dir.display()),
    }
}

fn remove_installed_binaries() {
    let binaries = &[
        "/usr/local/bin/nexus-link",
        "/usr/local/bin/nexus-link-agent",
        "/usr/local/bin/nexus-link-service",
    ];

    for bin in binaries {
        if Path::new(bin).exists() || std::fs::symlink_metadata(bin).is_ok() {
            let result = Command::new("sudo").args(["rm", "-f", bin]).output();
            match result {
                Ok(o) if o.status.success() => println!("  Removed: {}", bin),
                _ => println!("  Warning: could not remove {}", bin),
            }
        }
    }
}

fn remove_system_user() {
    // Check if user exists
    let check = Command::new("id").arg(SERVICE_USER).output();
    if let Ok(o) = check
        && !o.status.success()
    {
        return; // User doesn't exist, nothing to do
    }

    let result = Command::new("sudo")
        .args(["userdel", SERVICE_USER])
        .output();

    match result {
        Ok(o) if o.status.success() => {
            println!("  Removed system user: {}", SERVICE_USER)
        }
        _ => println!("  Warning: could not remove user {}", SERVICE_USER),
    }
}
