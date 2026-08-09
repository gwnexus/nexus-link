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
    let dir = Path::new(SYSTEM_CONFIG_DIR);

    if dir.exists() {
        return SetupStep {
            name: "Config directory",
            status: StepStatus::Skipped,
            message: format!("{} already exists", SYSTEM_CONFIG_DIR),
        };
    }

    let result = sudo(&["mkdir", "-p", SYSTEM_CONFIG_DIR])
        .and_then(|()| {
            sudo(&[
                "chown",
                &format!("{}:{}", SERVICE_USER, SERVICE_GROUP),
                SYSTEM_CONFIG_DIR,
            ])
        })
        .and_then(|()| sudo(&["chmod", "700", SYSTEM_CONFIG_DIR]));

    match result {
        Ok(()) => SetupStep {
            name: "Config directory",
            status: StepStatus::Created,
            message: format!(
                "Created {} (owner: {}, mode: 700)",
                SYSTEM_CONFIG_DIR, SERVICE_USER
            ),
        },
        Err(e) => SetupStep {
            name: "Config directory",
            status: StepStatus::Failed,
            message: format!("Failed: {}", e),
        },
    }
}

fn migrate_legacy_config() -> SetupStep {
    let legacy_path = config::default_config_path();
    let system_path = config::system_config_path();

    if system_path.exists() {
        return SetupStep {
            name: "Config migration",
            status: StepStatus::Skipped,
            message: "System config already exists — no migration needed".to_string(),
        };
    }

    if !legacy_path.exists() {
        return SetupStep {
            name: "Config migration",
            status: StepStatus::Skipped,
            message: "No legacy config found — will be created at registration".to_string(),
        };
    }

    let legacy_str = legacy_path.to_string_lossy().to_string();
    let system_str = system_path.to_string_lossy().to_string();

    let result = sudo(&["cp", "-p", &legacy_str, &system_str])
        .and_then(|()| {
            sudo(&[
                "chown",
                &format!("{}:{}", SERVICE_USER, SERVICE_GROUP),
                &system_str,
            ])
        })
        .and_then(|()| sudo(&["chmod", "600", &system_str]));

    // Also migrate signing_key.pub if present
    let legacy_key = config::dirs_home().join("signing_key.pub");
    let system_key = PathBuf::from(SYSTEM_CONFIG_DIR).join("signing_key.pub");
    if legacy_key.exists() && !system_key.exists() {
        let _ = sudo(&[
            "cp",
            "-p",
            &legacy_key.to_string_lossy(),
            &system_key.to_string_lossy(),
        ])
        .and_then(|()| {
            sudo(&[
                "chown",
                &format!("{}:{}", SERVICE_USER, SERVICE_GROUP),
                &system_key.to_string_lossy(),
            ])
        });
    }

    match result {
        Ok(()) => SetupStep {
            name: "Config migration",
            status: StepStatus::Created,
            message: format!("Migrated {} → {}", legacy_str, system_str),
        },
        Err(e) => SetupStep {
            name: "Config migration",
            status: StepStatus::Failed,
            message: format!("Failed: {}", e),
        },
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
