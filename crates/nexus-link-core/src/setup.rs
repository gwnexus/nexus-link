//! System-level setup for nexus-link services.
//!
//! Creates the dedicated `nexus-link` system user, service directories,
//! and migrates legacy user-level configs to the system path.
//! All privileged operations are executed via `sudo` internally.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{self, SERVICE_GROUP, SERVICE_USER, SYSTEM_CONFIG_DIR};

/// Result of the setup process.
#[derive(Debug)]
pub struct SetupReport {
    pub steps: Vec<SetupStep>,
    pub success: bool,
}

#[derive(Debug)]
pub struct SetupStep {
    pub name: &'static str,
    pub status: StepStatus,
    pub message: String,
}

#[derive(Debug, PartialEq)]
pub enum StepStatus {
    Created,
    Skipped,
    Failed,
}

/// Run the full system setup. Returns a report of all steps.
/// This function calls `sudo` internally for privileged operations.
pub fn run_setup() -> SetupReport {
    let steps = vec![
        create_system_user(),
        add_docker_group(),
        create_config_dir(),
        migrate_legacy_config(),
        install_binaries(),
        install_systemd_units(),
        enable_services(),
    ];

    let success = steps.iter().all(|s| s.status != StepStatus::Failed);
    SetupReport { steps, success }
}

/// Print a human-readable setup report.
pub fn print_report(report: &SetupReport) {
    println!();
    println!("System Setup Report:");
    println!("{}", "-".repeat(60));

    for step in &report.steps {
        let icon = match step.status {
            StepStatus::Created => "+",
            StepStatus::Skipped => "~",
            StepStatus::Failed => "!",
        };
        println!("  [{}] {}: {}", icon, step.name, step.message);
    }

    println!("{}", "-".repeat(60));
    if report.success {
        println!(
            "  Setup complete. Services will run as user '{}'.",
            SERVICE_USER
        );
        println!("  Config path: {}/config.toml", SYSTEM_CONFIG_DIR);
    } else {
        println!("  Setup incomplete — check failed steps above.");
    }
    println!();
}

// ── Step implementations ────────────────────────────────────────────────────

fn create_system_user() -> SetupStep {
    // Check if user already exists
    let check = Command::new("id").arg(SERVICE_USER).output();
    if let Ok(o) = check
        && o.status.success()
    {
        return SetupStep {
            name: "System user",
            status: StepStatus::Skipped,
            message: format!("User '{}' already exists", SERVICE_USER),
        };
    }

    let result = sudo(&[
        "useradd",
        "--system",
        "--no-create-home",
        "--shell",
        "/usr/sbin/nologin",
        "--home-dir",
        SYSTEM_CONFIG_DIR,
        SERVICE_USER,
    ]);

    match result {
        Ok(()) => SetupStep {
            name: "System user",
            status: StepStatus::Created,
            message: format!("Created system user '{}'", SERVICE_USER),
        },
        Err(e) => SetupStep {
            name: "System user",
            status: StepStatus::Failed,
            message: format!("Failed to create user: {}", e),
        },
    }
}

fn add_docker_group() -> SetupStep {
    // Check if docker group exists
    let group_check = Command::new("getent").args(["group", "docker"]).output();

    match group_check {
        Ok(o) if o.status.success() => {}
        _ => {
            return SetupStep {
                name: "Docker group",
                status: StepStatus::Skipped,
                message: "Docker group does not exist (docker not installed?)".to_string(),
            };
        }
    }

    // Check if user is already in docker group
    let id_check = Command::new("id").args(["-nG", SERVICE_USER]).output();
    if let Ok(o) = id_check {
        let groups = String::from_utf8_lossy(&o.stdout);
        if groups.split_whitespace().any(|g| g == "docker") {
            return SetupStep {
                name: "Docker group",
                status: StepStatus::Skipped,
                message: format!("User '{}' already in docker group", SERVICE_USER),
            };
        }
    }

    let result = sudo(&["usermod", "-aG", "docker", SERVICE_USER]);

    match result {
        Ok(()) => SetupStep {
            name: "Docker group",
            status: StepStatus::Created,
            message: format!("Added '{}' to docker group", SERVICE_USER),
        },
        Err(e) => SetupStep {
            name: "Docker group",
            status: StepStatus::Failed,
            message: format!("Failed to add to docker group: {}", e),
        },
    }
}

fn create_config_dir() -> SetupStep {
    let config_dir = Path::new(SYSTEM_CONFIG_DIR);
    let state_dir = Path::new(config::SYSTEM_STATE_DIR);

    let mut created = Vec::new();

    // Create /etc/nexus-link/ (root-owned, readable by service)
    if !config_dir.exists() {
        let result = sudo(&["mkdir", "-p", SYSTEM_CONFIG_DIR])
            .and_then(|()| sudo(&["chown", "root:root", SYSTEM_CONFIG_DIR]))
            .and_then(|()| sudo(&["chmod", "755", SYSTEM_CONFIG_DIR]));
        match result {
            Ok(()) => created.push(SYSTEM_CONFIG_DIR),
            Err(e) => {
                return SetupStep {
                    name: "Config directory",
                    status: StepStatus::Failed,
                    message: format!("Failed to create {}: {}", SYSTEM_CONFIG_DIR, e),
                };
            }
        }
    }

    // Create /var/lib/nexus-link/ (service-owned, for runtime state/keys)
    if !state_dir.exists() {
        let result = sudo(&["mkdir", "-p", config::SYSTEM_STATE_DIR])
            .and_then(|()| {
                sudo(&[
                    "chown",
                    &format!("{}:{}", SERVICE_USER, SERVICE_GROUP),
                    config::SYSTEM_STATE_DIR,
                ])
            })
            .and_then(|()| sudo(&["chmod", "700", config::SYSTEM_STATE_DIR]));
        match result {
            Ok(()) => created.push(config::SYSTEM_STATE_DIR),
            Err(e) => {
                return SetupStep {
                    name: "Config directory",
                    status: StepStatus::Failed,
                    message: format!("Failed to create {}: {}", config::SYSTEM_STATE_DIR, e),
                };
            }
        }
    }

    if created.is_empty() {
        SetupStep {
            name: "Directories",
            status: StepStatus::Skipped,
            message: format!(
                "{} and {} already exist",
                SYSTEM_CONFIG_DIR,
                config::SYSTEM_STATE_DIR
            ),
        }
    } else {
        SetupStep {
            name: "Directories",
            status: StepStatus::Created,
            message: format!("Created: {}", created.join(", ")),
        }
    }
}

fn migrate_legacy_config() -> SetupStep {
    let system_path = config::system_config_path();

    if system_path.exists() {
        return SetupStep {
            name: "Config migration",
            status: StepStatus::Skipped,
            message: "System config already exists — no migration needed".to_string(),
        };
    }

    // Check migration sources in priority order:
    // 1. /var/lib/nexus-link/config.toml (from v0.10.0/v0.10.1 setup)
    // 2. ~/.nexus-link/config.toml (original legacy path)
    let var_lib_config = PathBuf::from(config::SYSTEM_STATE_DIR).join("config.toml");
    let legacy_path = config::default_config_path();

    let source = if var_lib_config.exists() {
        var_lib_config
    } else if legacy_path.exists() {
        legacy_path
    } else {
        return SetupStep {
            name: "Config migration",
            status: StepStatus::Skipped,
            message: "No legacy config found — will be created at registration".to_string(),
        };
    };

    let source_str = source.to_string_lossy().to_string();
    let system_str = system_path.to_string_lossy().to_string();

    let result = sudo(&["cp", "-p", &source_str, &system_str])
        .and_then(|()| sudo(&["chown", "root:root", &system_str]))
        .and_then(|()| sudo(&["chmod", "644", &system_str]));

    // Migrate signing_key.pub to state directory
    let legacy_key_locations = [
        config::dirs_home().join("signing_key.pub"),
        PathBuf::from(config::SYSTEM_STATE_DIR).join("signing_key.pub"),
    ];
    let state_key = PathBuf::from(config::SYSTEM_STATE_DIR).join("signing_key.pub");
    if !state_key.exists() {
        for key_src in &legacy_key_locations {
            if key_src.exists() {
                let _ = sudo(&[
                    "cp",
                    "-p",
                    &key_src.to_string_lossy(),
                    &state_key.to_string_lossy(),
                ])
                .and_then(|()| {
                    sudo(&[
                        "chown",
                        &format!("{}:{}", SERVICE_USER, SERVICE_GROUP),
                        &state_key.to_string_lossy(),
                    ])
                });
                break;
            }
        }
    }

    match result {
        Ok(()) => SetupStep {
            name: "Config migration",
            status: StepStatus::Created,
            message: format!("Migrated {} → {}", source_str, system_str),
        },
        Err(e) => SetupStep {
            name: "Config migration",
            status: StepStatus::Failed,
            message: format!("Failed: {}", e),
        },
    }
}

// ── Binary installation ─────────────────────────────────────────────────────

/// Target directory for system-wide binary installation.
const INSTALL_DIR: &str = "/usr/local/bin";

/// Binary names shipped by nexus-link.
const BINARIES: &[&str] = &["nexus-link", "nexus-link-agent", "nexus-link-service"];

fn install_binaries() -> SetupStep {
    // Resolve the currently running executable to find the source directory.
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return SetupStep {
                name: "Install binaries",
                status: StepStatus::Failed,
                message: format!("Cannot resolve current executable: {}", e),
            };
        }
    };

    // Canonicalize to resolve symlinks (e.g. /usr/local/bin/nexus-link -> ~/.local/bin/nexus-link)
    let real_exe = match current_exe.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return SetupStep {
                name: "Install binaries",
                status: StepStatus::Failed,
                message: format!("Cannot canonicalize executable path: {}", e),
            };
        }
    };

    let source_dir = match real_exe.parent() {
        Some(d) => d.to_path_buf(),
        None => {
            return SetupStep {
                name: "Install binaries",
                status: StepStatus::Failed,
                message: "Cannot determine source binary directory".to_string(),
            };
        }
    };

    let install_dir = Path::new(INSTALL_DIR);

    // If source == target, binaries are already in the install dir.
    // Check common user-local paths as alternative source (post-upgrade scenario).
    let effective_source = if source_dir == install_dir {
        find_user_binary_dir().unwrap_or(source_dir)
    } else {
        source_dir
    };

    // If still the same, binaries are already correctly installed
    if effective_source == install_dir {
        return SetupStep {
            name: "Install binaries",
            status: StepStatus::Skipped,
            message: "Binaries already in /usr/local/bin (up to date)".to_string(),
        };
    }

    let mut installed = Vec::new();
    let mut errors = Vec::new();

    for bin_name in BINARIES {
        let source = effective_source.join(bin_name);
        let target = install_dir.join(bin_name);

        if !source.exists() {
            // Binary may not be present (e.g. single-binary installs) — skip
            continue;
        }

        // Skip if source and target are the same file
        if let (Ok(s), Ok(t)) = (source.canonicalize(), target.canonicalize())
            && s == t
        {
            continue;
        }

        // Remove existing symlink or file at target
        if (target.exists() || target.symlink_metadata().is_ok())
            && let Err(e) = sudo(&["rm", "-f", &target.to_string_lossy()])
        {
            errors.push(format!("{}: {}", bin_name, e));
            continue;
        }

        // Copy binary to system path
        let result = sudo(&["cp", &source.to_string_lossy(), &target.to_string_lossy()])
            .and_then(|()| sudo(&["chmod", "755", &target.to_string_lossy()]));

        match result {
            Ok(()) => installed.push(*bin_name),
            Err(e) => errors.push(format!("{}: {}", bin_name, e)),
        }
    }

    if !errors.is_empty() {
        SetupStep {
            name: "Install binaries",
            status: StepStatus::Failed,
            message: format!("Errors: {}", errors.join("; ")),
        }
    } else if installed.is_empty() {
        SetupStep {
            name: "Install binaries",
            status: StepStatus::Skipped,
            message: "No binaries found in source directory".to_string(),
        }
    } else {
        SetupStep {
            name: "Install binaries",
            status: StepStatus::Created,
            message: format!(
                "Installed {} → {} ({})",
                installed.len(),
                INSTALL_DIR,
                installed.join(", ")
            ),
        }
    }
}

/// Search common user-local binary directories for nexus-link binaries.
/// Used when the running binary is already in /usr/local/bin/ (post-upgrade
/// installs new versions to ~/.local/bin/ but the running process is the old one).
fn find_user_binary_dir() -> Option<PathBuf> {
    // Under sudo, HOME is /root but SUDO_USER has the real invoking user.
    // Check SUDO_USER first to find binaries installed by the operator.
    let invoking_home = std::env::var("SUDO_USER")
        .map(|u| format!("/home/{}", u))
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "/root".to_string());

    let candidates = [
        PathBuf::from(&invoking_home).join(".local/bin"),
        PathBuf::from(&invoking_home).join(".cargo/bin"),
        // Also check /root/.local/bin in case the install was done as root
        PathBuf::from("/root/.local/bin"),
    ];

    for dir in &candidates {
        if dir.join("nexus-link").exists() && dir != Path::new(INSTALL_DIR) {
            return Some(dir.clone());
        }
    }

    None
}

// ── Systemd units ───────────────────────────────────────────────────────────

const SYSTEMD_DIR: &str = "/etc/systemd/system";

const AGENT_UNIT: &str = r#"[Unit]
Description=Nexus Link Agent — telemetry push daemon
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
User=nexus-link
Group=nexus-link
SupplementaryGroups=docker
ExecStart=/usr/local/bin/nexus-link-agent
Restart=on-failure
RestartSec=5
Environment=NEXUS_LINK_CONFIG=/etc/nexus-link/config.toml

# Hardening
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
NoNewPrivileges=true
ReadWritePaths=/var/lib/nexus-link

[Install]
WantedBy=multi-user.target
"#;

const SERVICE_UNIT: &str = r#"[Unit]
Description=Nexus Link Service — command receiver (HTTPS :8443)
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
Type=simple
User=nexus-link
Group=nexus-link
SupplementaryGroups=docker
ExecStart=/usr/local/bin/nexus-link-service
Restart=on-failure
RestartSec=5
Environment=NEXUS_LINK_CONFIG=/etc/nexus-link/config.toml

# Hardening
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
NoNewPrivileges=true
ReadWritePaths=/var/lib/nexus-link

[Install]
WantedBy=multi-user.target
"#;

fn install_systemd_units() -> SetupStep {
    let agent_path = format!("{}/nexus-link-agent.service", SYSTEMD_DIR);
    let service_path = format!("{}/nexus-link-service.service", SYSTEMD_DIR);

    // Write unit files via tee (sudo)
    let result = write_file_sudo(&agent_path, AGENT_UNIT)
        .and_then(|()| write_file_sudo(&service_path, SERVICE_UNIT))
        .and_then(|()| sudo(&["systemctl", "daemon-reload"]));

    match result {
        Ok(()) => SetupStep {
            name: "Systemd units",
            status: StepStatus::Created,
            message: "Installed nexus-link-agent.service + nexus-link-service.service, daemon-reload done".to_string(),
        },
        Err(e) => SetupStep {
            name: "Systemd units",
            status: StepStatus::Failed,
            message: format!("Failed: {}", e),
        },
    }
}

fn enable_services() -> SetupStep {
    let result = sudo(&["systemctl", "enable", "--now", "nexus-link-agent.service"])
        .and_then(|()| sudo(&["systemctl", "enable", "--now", "nexus-link-service.service"]));

    match result {
        Ok(()) => SetupStep {
            name: "Enable services",
            status: StepStatus::Created,
            message: "Enabled and started nexus-link-agent + nexus-link-service".to_string(),
        },
        Err(e) => SetupStep {
            name: "Enable services",
            status: StepStatus::Failed,
            message: format!("Failed: {}", e),
        },
    }
}

/// Write content to a file via sudo tee.
fn write_file_sudo(path: &str, content: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("sudo")
        .args(["tee", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn sudo tee: {}", e))?;

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write to {}: {}", path, e))?;

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for sudo tee: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("sudo tee {} exited with {}", path, status))
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Run a command via sudo. Returns Ok(()) on success.
fn sudo(args: &[&str]) -> Result<(), String> {
    let output = Command::new("sudo")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to invoke sudo: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("sudo {} failed: {}", args.join(" "), stderr.trim()))
    }
}
