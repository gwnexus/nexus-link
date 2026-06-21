use nexus_link_core::config::{self, Config};
use std::path::PathBuf;
use tracing::info;

const AGENT_SERVICE_NAME: &str = "nexus-link-agent";
const SERVICE_SERVICE_NAME: &str = "nexus-link-service";

pub async fn start() -> anyhow::Result<()> {
    let config = Config::load()?;
    info!("Starting nexus-link agent and service...");

    // Check if systemd unit files exist, generate if not
    if !systemd_unit_exists(AGENT_SERVICE_NAME) {
        println!("Generating systemd unit files...");
        generate_systemd_units(&config)?;
        reload_systemd()?;
    }

    // Enable and start both services
    systemctl("enable", AGENT_SERVICE_NAME)?;
    systemctl("start", AGENT_SERVICE_NAME)?;
    println!(
        "  Agent:   started (telemetry push every {}s)",
        config.api.push_interval_secs
    );

    systemctl("enable", SERVICE_SERVICE_NAME)?;
    systemctl("start", SERVICE_SERVICE_NAME)?;
    println!(
        "  Service: started (listening on {}:{})",
        config.service.listen_addr, config.service.port
    );

    println!();
    println!("Both services enabled and started.");
    println!("  View logs: nexus-link agent logs");
    println!("  Stop:      nexus-link agent stop");

    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    info!("Stopping nexus-link agent and service...");

    let mut stopped = false;

    if systemctl("stop", AGENT_SERVICE_NAME).is_ok() {
        println!("  Agent:   stopped");
        stopped = true;
    }

    if systemctl("stop", SERVICE_SERVICE_NAME).is_ok() {
        println!("  Service: stopped");
        stopped = true;
    }

    if !stopped {
        // Fallback: try pkill
        let _ = std::process::Command::new("pkill")
            .args(["-f", "nexus-link-agent"])
            .output();
        let _ = std::process::Command::new("pkill")
            .args(["-f", "nexus-link-service"])
            .output();
        println!("  Processes stopped via signal (no systemd units found)");
    }

    Ok(())
}

pub async fn logs(tail: u32) -> anyhow::Result<()> {
    // Try journalctl first (systemd)
    let output = std::process::Command::new("journalctl")
        .args([
            "-u",
            AGENT_SERVICE_NAME,
            "-u",
            SERVICE_SERVICE_NAME,
            "-n",
            &tail.to_string(),
            "--no-pager",
            "-o",
            "short-iso",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            print!("{}", stdout);
        }
        _ => {
            println!("journalctl not available. Try:");
            println!(
                "  journalctl -u {} -n {} --no-pager",
                AGENT_SERVICE_NAME, tail
            );
            println!(
                "  journalctl -u {} -n {} --no-pager",
                SERVICE_SERVICE_NAME, tail
            );
        }
    }

    Ok(())
}

/// Check if a systemd unit file exists
fn systemd_unit_exists(service_name: &str) -> bool {
    let path = systemd_unit_path(service_name);
    path.exists()
}

/// Get the path for a systemd user/system unit
fn systemd_unit_path(service_name: &str) -> PathBuf {
    PathBuf::from(format!("/etc/systemd/system/{}.service", service_name))
}

/// Generate systemd unit files for agent and service
fn generate_systemd_units(config: &Config) -> anyhow::Result<()> {
    let nexus_link_bin = which_binary("nexus-link-agent")?;
    let nexus_link_service_bin = which_binary("nexus-link-service")?;
    let config_path = config::default_config_path();

    // Agent unit
    let agent_unit = format!(
        r#"[Unit]
Description=Nexus Link Telemetry Agent
Documentation=https://github.com/gwnexus/nexus-link
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
ExecStart={bin}
Restart=always
RestartSec=10
Environment=RUST_LOG=nexus_link_agent=info
WorkingDirectory={home}

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadOnlyPaths=/
ReadWritePaths={home}
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"#,
        bin = nexus_link_bin.display(),
        home = config_path
            .parent()
            .unwrap_or(&PathBuf::from("/root"))
            .display(),
    );

    // Service unit
    let service_unit = format!(
        r#"[Unit]
Description=Nexus Link Command Service
Documentation=https://github.com/gwnexus/nexus-link
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin}
Restart=always
RestartSec=10
Environment=RUST_LOG=nexus_link_service=info
WorkingDirectory={home}

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadOnlyPaths=/
ReadWritePaths={home}
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"#,
        bin = nexus_link_service_bin.display(),
        home = config_path
            .parent()
            .unwrap_or(&PathBuf::from("/root"))
            .display(),
    );

    // Write unit files (requires root/sudo)
    let agent_path = systemd_unit_path(AGENT_SERVICE_NAME);
    let service_path = systemd_unit_path(SERVICE_SERVICE_NAME);

    write_unit_file(&agent_path, &agent_unit)?;
    write_unit_file(&service_path, &service_unit)?;

    println!("  Generated: {}", agent_path.display());
    println!("  Generated: {}", service_path.display());

    Ok(())
}

/// Write a systemd unit file, attempting sudo if permission denied
fn write_unit_file(path: &PathBuf, content: &str) -> anyhow::Result<()> {
    match std::fs::write(path, content) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            // Try via sudo tee
            let mut child = std::process::Command::new("sudo")
                .args(["tee", &path.to_string_lossy()])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()?;

            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                stdin.write_all(content.as_bytes())?;
            }

            let status = child.wait()?;
            if status.success() {
                Ok(())
            } else {
                anyhow::bail!(
                    "Failed to write {} (sudo returned {}). Run as root or with sudo.",
                    path.display(),
                    status
                )
            }
        }
        Err(e) => anyhow::bail!("Failed to write {}: {}", path.display(), e),
    }
}

/// Find the path of an installed binary
fn which_binary(name: &str) -> anyhow::Result<PathBuf> {
    // Check common locations
    let candidates = [
        PathBuf::from(format!("/usr/local/bin/{}", name)),
        PathBuf::from(format!("/usr/bin/{}", name)),
        dirs_local_bin().join(name),
    ];

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Fallback: use `which`
    let output = std::process::Command::new("which").arg(name).output()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok(PathBuf::from(path));
    }

    anyhow::bail!(
        "Binary '{}' not found. Ensure nexus-link is installed in PATH.",
        name
    )
}

fn dirs_local_bin() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/bin")
}

/// Run systemctl with a command and service name
fn systemctl(command: &str, service: &str) -> anyhow::Result<()> {
    let output = std::process::Command::new("systemctl")
        .args([command, service])
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        // Try with sudo
        let output = std::process::Command::new("sudo")
            .args(["systemctl", command, service])
            .output()?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "systemctl {} {} failed: {}",
                command,
                service,
                stderr.trim()
            )
        }
    }
}

/// Reload systemd daemon
fn reload_systemd() -> anyhow::Result<()> {
    let output = std::process::Command::new("systemctl")
        .args(["daemon-reload"])
        .output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => {
            let _ = std::process::Command::new("sudo")
                .args(["systemctl", "daemon-reload"])
                .output();
            Ok(())
        }
    }
}
