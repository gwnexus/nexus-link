use nexus_link_core::config::{self, Config};
use std::path::PathBuf;
use tracing::info;

const AGENT_SERVICE_NAME: &str = "nexus-link-agent";
const SERVICE_SERVICE_NAME: &str = "nexus-link-service";

/// Detect whether we're running as root
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Check if the current user can use sudo (non-interactively)
fn has_sudo() -> bool {
    std::process::Command::new("sudo")
        .args(["-n", "true"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Determine the systemd mode: system (root/sudo) or user (non-root)
#[derive(Debug, Clone, Copy, PartialEq)]
enum SystemdMode {
    /// System-wide units in /etc/systemd/system/ (root or sudo)
    System,
    /// Per-user units in ~/.config/systemd/user/ (no root needed)
    User,
}

fn detect_systemd_mode() -> SystemdMode {
    if is_root() || has_sudo() {
        SystemdMode::System
    } else {
        SystemdMode::User
    }
}

pub async fn start() -> anyhow::Result<()> {
    let config = Config::load()?;
    let mode = detect_systemd_mode();

    info!(
        "Starting nexus-link agent and service (mode: {:?})...",
        mode
    );

    if mode == SystemdMode::User {
        println!("Running as non-root user -- using systemd user units.");
        println!("  Note: user services require 'loginctl enable-linger $USER'");
        println!();
    }

    // Check if systemd unit files exist, generate if not
    if !systemd_unit_exists(AGENT_SERVICE_NAME, mode) {
        println!("Generating systemd unit files...");
        generate_systemd_units(mode)?;
        reload_systemd(mode)?;
    }

    // Enable and start both services
    systemctl("enable", AGENT_SERVICE_NAME, mode)?;
    systemctl("start", AGENT_SERVICE_NAME, mode)?;
    println!(
        "  Agent:   started (telemetry push every {}s)",
        config.agent.push_sec
    );

    systemctl("enable", SERVICE_SERVICE_NAME, mode)?;
    systemctl("start", SERVICE_SERVICE_NAME, mode)?;
    println!(
        "  Service: started (listening on {}:{})",
        config.service.listen_addr, config.service.port
    );

    println!();
    println!("Both services enabled and started.");
    println!("  View logs: nexus-link agent logs");
    println!("  Stop:      nexus-link agent stop");

    if mode == SystemdMode::User {
        // Check linger status
        let user = std::env::var("USER").unwrap_or_default();
        let linger_check = std::process::Command::new("loginctl")
            .args(["show-user", &user, "--property=Linger"])
            .output();

        let needs_linger = match linger_check {
            Ok(o) if o.status.success() => {
                let out = String::from_utf8_lossy(&o.stdout);
                !out.contains("Linger=yes")
            }
            _ => true,
        };

        if needs_linger {
            println!();
            println!("  WARNING: User linger not enabled. Services will stop on logout.");
            println!("  Run: sudo loginctl enable-linger {}", user);
        }
    }

    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    info!("Stopping nexus-link agent and service...");

    let mode = detect_systemd_mode();
    let mut stopped = false;

    if systemctl("stop", AGENT_SERVICE_NAME, mode).is_ok() {
        println!("  Agent:   stopped");
        stopped = true;
    }

    if systemctl("stop", SERVICE_SERVICE_NAME, mode).is_ok() {
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
    let mode = detect_systemd_mode();
    let mut args = vec![
        "-u".to_string(),
        AGENT_SERVICE_NAME.to_string(),
        "-u".to_string(),
        SERVICE_SERVICE_NAME.to_string(),
        "-n".to_string(),
        tail.to_string(),
        "--no-pager".to_string(),
        "-o".to_string(),
        "short-iso".to_string(),
    ];

    if mode == SystemdMode::User {
        args.push("--user".to_string());
    }

    let output = std::process::Command::new("journalctl")
        .args(&args)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            print!("{}", stdout);
        }
        _ => {
            let flag = if mode == SystemdMode::User {
                " --user"
            } else {
                ""
            };
            println!("journalctl not available. Try:");
            println!(
                "  journalctl{} -u {} -n {} --no-pager",
                flag, AGENT_SERVICE_NAME, tail
            );
        }
    }

    Ok(())
}

/// Check if a systemd unit file exists
fn systemd_unit_exists(service_name: &str, mode: SystemdMode) -> bool {
    systemd_unit_path(service_name, mode).exists()
}

/// Get the path for a systemd unit file based on mode
fn systemd_unit_path(service_name: &str, mode: SystemdMode) -> PathBuf {
    match mode {
        SystemdMode::System => {
            PathBuf::from(format!("/etc/systemd/system/{}.service", service_name))
        }
        SystemdMode::User => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            PathBuf::from(home)
                .join(".config/systemd/user")
                .join(format!("{}.service", service_name))
        }
    }
}

/// Generate systemd unit files for agent and service
fn generate_systemd_units(mode: SystemdMode) -> anyhow::Result<()> {
    let nexus_link_bin = which_binary("nexus-link-agent")?;
    let nexus_link_service_bin = which_binary("nexus-link-service")?;
    let config_path = config::default_config_path();
    let working_dir = config_path
        .parent()
        .unwrap_or(&PathBuf::from("/root"))
        .display()
        .to_string();

    // For system units running as non-root user, we need User= and HOME=
    let current_user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
    let current_home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());

    let (user_directive, install_target, extra_hardening) = match mode {
        SystemdMode::System => {
            let user_line = if current_user != "root" {
                format!(
                    "User={}\nGroup={}\nEnvironment=HOME={}",
                    current_user, current_user, current_home
                )
            } else {
                String::new()
            };
            (
                user_line,
                "WantedBy=multi-user.target".to_string(),
                format!(
                    "\
# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadOnlyPaths=/
ReadWritePaths={}
PrivateTmp=true",
                    working_dir
                ),
            )
        }
        SystemdMode::User => (
            String::new(),
            "WantedBy=default.target".to_string(),
            String::new(),
        ),
    };

    // Agent unit
    let agent_unit = format!(
        r#"[Unit]
Description=Nexus Link Telemetry Agent
Documentation=https://github.com/gwnexus/nexus-link
After=network-online.target docker.service
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin}
Restart=always
RestartSec=10
Environment=RUST_LOG=nexus_link_agent=info
WorkingDirectory={home}
{user}
{extra}

[Install]
{install}
"#,
        bin = nexus_link_bin.display(),
        home = working_dir,
        user = user_directive,
        extra = extra_hardening,
        install = install_target,
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
{user}
{extra}

[Install]
{install}
"#,
        bin = nexus_link_service_bin.display(),
        home = working_dir,
        user = user_directive,
        extra = extra_hardening,
        install = install_target,
    );

    let agent_path = systemd_unit_path(AGENT_SERVICE_NAME, mode);
    let service_path = systemd_unit_path(SERVICE_SERVICE_NAME, mode);

    // Ensure directory exists for user units
    if mode == SystemdMode::User
        && let Some(parent) = agent_path.parent()
    {
        std::fs::create_dir_all(parent)?;
    }

    write_unit_file(&agent_path, &agent_unit, mode)?;
    write_unit_file(&service_path, &service_unit, mode)?;

    println!("  Generated: {}", agent_path.display());
    println!("  Generated: {}", service_path.display());

    Ok(())
}

/// Write a systemd unit file, handling permissions based on mode
fn write_unit_file(path: &PathBuf, content: &str, mode: SystemdMode) -> anyhow::Result<()> {
    match mode {
        SystemdMode::User => {
            // User mode: write directly, no sudo needed
            std::fs::write(path, content)?;
            Ok(())
        }
        SystemdMode::System => {
            // System mode: try direct write, fall back to sudo
            match std::fs::write(path, content) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
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
                            "Failed to write {} -- sudo required. Run: sudo nexus-link agent start",
                            path.display()
                        )
                    }
                }
                Err(e) => anyhow::bail!("Failed to write {}: {}", path.display(), e),
            }
        }
    }
}

/// Find the path of an installed binary
fn which_binary(name: &str) -> anyhow::Result<PathBuf> {
    let candidates = [
        dirs_local_bin().join(name),
        PathBuf::from(format!("/usr/local/bin/{}", name)),
        PathBuf::from(format!("/usr/bin/{}", name)),
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

/// Run systemctl with a command and service name, respecting mode
fn systemctl(command: &str, service: &str, mode: SystemdMode) -> anyhow::Result<()> {
    let mut args: Vec<&str> = vec![];

    if mode == SystemdMode::User {
        args.push("--user");
    }
    args.push(command);
    args.push(service);

    let output = std::process::Command::new("systemctl")
        .args(&args)
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    // System mode: retry with sudo
    if mode == SystemdMode::System {
        let mut sudo_args = vec!["systemctl"];
        sudo_args.extend_from_slice(&args);

        let output = std::process::Command::new("sudo")
            .args(&sudo_args)
            .output()?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "systemctl {} {} failed: {}",
            command,
            service,
            stderr.trim()
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "systemctl --user {} {} failed: {}",
        command,
        service,
        stderr.trim()
    )
}

/// Reload systemd daemon
fn reload_systemd(mode: SystemdMode) -> anyhow::Result<()> {
    let args: Vec<&str> = match mode {
        SystemdMode::User => vec!["--user", "daemon-reload"],
        SystemdMode::System => vec!["daemon-reload"],
    };

    let output = std::process::Command::new("systemctl").args(&args).output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ if mode == SystemdMode::System => {
            let _ = std::process::Command::new("sudo")
                .args(["systemctl", "daemon-reload"])
                .output();
            Ok(())
        }
        _ => Ok(()),
    }
}
