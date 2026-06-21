use serde::{Deserialize, Serialize};
use std::process::Command;
use tracing::{info, warn};

/// Result of the device preflight check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub checks: Vec<PreflightCheck>,
    pub verdict: PreflightVerdict,
    pub device_match: Option<KnownDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreflightVerdict {
    /// Device is fully compatible and listed as a known/tested device
    Compatible,
    /// Device has GPU and meets requirements but is not a known/tested device
    NotRecommended,
    /// Device lacks GPU or critical requirements -- linking not advised
    Incompatible,
}

/// Known devices that are fully tested and supported
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDevice {
    pub name: String,
    pub identifier: String,
    pub notes: String,
}

/// Known device registry -- devices that are 100% compatible
const KNOWN_DEVICES: &[(&str, &str, &str)] = &[
    (
        "NVIDIA DGX Spark",
        "gb10",
        "GB10 Grace Blackwell, 128 GB unified memory, aarch64",
    ),
    (
        "NVIDIA DGX Station A100",
        "a100",
        "4x A100 80GB, AMD EPYC, x86_64",
    ),
    (
        "NVIDIA DGX A100",
        "dgx-a100",
        "8x A100 80GB, AMD EPYC, x86_64",
    ),
    ("NVIDIA DGX H100", "dgx-h100", "8x H100 80GB, x86_64"),
];

/// Run the full preflight check suite for device compatibility
pub fn run_preflight() -> PreflightReport {
    let mut checks = Vec::new();

    // 1. Architecture check
    checks.push(check_architecture());

    // 2. GPU detection
    let gpu_check = check_gpu();
    let has_gpu = gpu_check.status == CheckStatus::Pass;
    checks.push(gpu_check);

    // 3. Docker availability
    checks.push(check_docker());

    // 4. Network connectivity (basic)
    checks.push(check_network());

    // 5. Disk space
    checks.push(check_disk_space());

    // 6. Known device matching
    let device_match = detect_known_device();

    // Determine verdict
    let verdict = if device_match.is_some() {
        PreflightVerdict::Compatible
    } else if has_gpu {
        PreflightVerdict::NotRecommended
    } else {
        PreflightVerdict::Incompatible
    };

    PreflightReport {
        checks,
        verdict,
        device_match,
    }
}

/// Check system architecture (aarch64 or x86_64 expected)
fn check_architecture() -> PreflightCheck {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let (status, detail) = match (os, arch) {
        ("linux", "aarch64") => (CheckStatus::Pass, format!("{}/{} (ARM64 Linux)", os, arch)),
        ("linux", "x86_64") => (CheckStatus::Pass, format!("{}/{} (x86_64 Linux)", os, arch)),
        ("linux", _) => (
            CheckStatus::Warn,
            format!("{}/{} (uncommon architecture)", os, arch),
        ),
        (_, _) => (
            CheckStatus::Warn,
            format!("{}/{} (non-Linux -- development use only)", os, arch),
        ),
    };

    PreflightCheck {
        name: "architecture".to_string(),
        status,
        detail,
    }
}

/// Detect GPU via nvidia-smi
fn check_gpu() -> PreflightCheck {
    match Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total,driver_version")
        .arg("--format=csv,noheader,nounits")
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let first_line = stdout.lines().next().unwrap_or("unknown GPU");
            let gpu_count = stdout.lines().count();

            PreflightCheck {
                name: "gpu".to_string(),
                status: CheckStatus::Pass,
                detail: format!("{} GPU(s) detected: {}", gpu_count, first_line.trim()),
            }
        }
        Ok(_) => PreflightCheck {
            name: "gpu".to_string(),
            status: CheckStatus::Fail,
            detail: "nvidia-smi found but returned error (no NVIDIA GPU or driver issue)"
                .to_string(),
        },
        Err(_) => PreflightCheck {
            name: "gpu".to_string(),
            status: CheckStatus::Fail,
            detail: "nvidia-smi not found -- no NVIDIA GPU detected".to_string(),
        },
    }
}

/// Check Docker availability
fn check_docker() -> PreflightCheck {
    match Command::new("docker").arg("info").output() {
        Ok(output) if output.status.success() => PreflightCheck {
            name: "docker".to_string(),
            status: CheckStatus::Pass,
            detail: "Docker daemon accessible".to_string(),
        },
        Ok(_) => PreflightCheck {
            name: "docker".to_string(),
            status: CheckStatus::Warn,
            detail: "Docker found but daemon not accessible (permission issue?)".to_string(),
        },
        Err(_) => PreflightCheck {
            name: "docker".to_string(),
            status: CheckStatus::Warn,
            detail: "Docker not found (container metrics will be unavailable)".to_string(),
        },
    }
}

/// Basic network check (can we reach the Nexus API?)
fn check_network() -> PreflightCheck {
    match Command::new("curl")
        .args([
            "-sfSL",
            "--max-time",
            "5",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
        ])
        .arg("https://nexus.gatewarden.eu/api/health")
        .output()
    {
        Ok(output) if output.status.success() => {
            let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if code == "200" {
                PreflightCheck {
                    name: "network".to_string(),
                    status: CheckStatus::Pass,
                    detail: "Nexus API reachable".to_string(),
                }
            } else {
                PreflightCheck {
                    name: "network".to_string(),
                    status: CheckStatus::Warn,
                    detail: format!("Nexus API returned HTTP {}", code),
                }
            }
        }
        _ => PreflightCheck {
            name: "network".to_string(),
            status: CheckStatus::Warn,
            detail: "Cannot reach Nexus API (offline or firewall)".to_string(),
        },
    }
}

/// Check available disk space (warn below 10GB)
fn check_disk_space() -> PreflightCheck {
    match Command::new("df")
        .args(["-BG", "--output=avail", "/"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let avail_str = stdout
                .lines()
                .nth(1)
                .unwrap_or("0G")
                .trim()
                .trim_end_matches('G');
            let avail_gb: u64 = avail_str.parse().unwrap_or(0);

            if avail_gb >= 10 {
                PreflightCheck {
                    name: "disk".to_string(),
                    status: CheckStatus::Pass,
                    detail: format!("{}G available", avail_gb),
                }
            } else {
                PreflightCheck {
                    name: "disk".to_string(),
                    status: CheckStatus::Warn,
                    detail: format!("Only {}G available (recommend >= 10G)", avail_gb),
                }
            }
        }
        _ => {
            // macOS fallback
            match Command::new("df").args(["-g", "/"]).output() {
                Ok(output) if output.status.success() => PreflightCheck {
                    name: "disk".to_string(),
                    status: CheckStatus::Pass,
                    detail: "Disk space check passed (macOS)".to_string(),
                },
                _ => PreflightCheck {
                    name: "disk".to_string(),
                    status: CheckStatus::Warn,
                    detail: "Could not determine available disk space".to_string(),
                },
            }
        }
    }
}

/// Try to match against known/tested devices
fn detect_known_device() -> Option<KnownDevice> {
    // Strategy: read /sys/firmware/devicetree/base/model (ARM)
    // or check nvidia-smi output for known GPU names
    // or check DMI product name

    // Try device tree (ARM boards like DGX Spark)
    if let Ok(model) = std::fs::read_to_string("/sys/firmware/devicetree/base/model") {
        let model_lower = model.to_lowercase();
        for (name, id, notes) in KNOWN_DEVICES {
            if model_lower.contains(&id.to_lowercase())
                || model_lower.contains(&name.to_lowercase())
            {
                info!("Known device matched via device tree: {}", name);
                return Some(KnownDevice {
                    name: name.to_string(),
                    identifier: id.to_string(),
                    notes: notes.to_string(),
                });
            }
        }
    }

    // Try DMI product name (x86_64 servers)
    if let Ok(product) = std::fs::read_to_string("/sys/class/dmi/id/product_name") {
        let product_lower = product.to_lowercase();
        for (name, id, notes) in KNOWN_DEVICES {
            if product_lower.contains(&id.to_lowercase())
                || product_lower.contains(&name.to_lowercase())
            {
                info!("Known device matched via DMI: {}", name);
                return Some(KnownDevice {
                    name: name.to_string(),
                    identifier: id.to_string(),
                    notes: notes.to_string(),
                });
            }
        }
    }

    // Try nvidia-smi GPU name matching
    if let Ok(output) = Command::new("nvidia-smi")
        .arg("--query-gpu=name")
        .arg("--format=csv,noheader")
        .output()
        && output.status.success()
    {
        let gpu_name = String::from_utf8_lossy(&output.stdout).to_lowercase();
        // DGX Spark uses GB10 Grace Blackwell
        if gpu_name.contains("blackwell") || gpu_name.contains("gb10") || gpu_name.contains("gb202")
        {
            return Some(KnownDevice {
                name: "NVIDIA DGX Spark".to_string(),
                identifier: "gb10".to_string(),
                notes: "GB10 Grace Blackwell, 128 GB unified memory, aarch64".to_string(),
            });
        }
        if gpu_name.contains("a100") {
            return Some(KnownDevice {
                name: "NVIDIA DGX Station A100".to_string(),
                identifier: "a100".to_string(),
                notes: "A100 GPU detected".to_string(),
            });
        }
        if gpu_name.contains("h100") {
            return Some(KnownDevice {
                name: "NVIDIA DGX H100".to_string(),
                identifier: "dgx-h100".to_string(),
                notes: "H100 GPU detected".to_string(),
            });
        }
    }

    warn!("No known device match found");
    None
}

/// Print the preflight report to stdout in a human-readable format
pub fn print_report(report: &PreflightReport) {
    println!();
    println!("  Nexus Link -- Device Preflight Check");
    println!("  =====================================");
    println!();

    for check in &report.checks {
        let icon = match check.status {
            CheckStatus::Pass => "\x1b[32m[PASS]\x1b[0m",
            CheckStatus::Warn => "\x1b[33m[WARN]\x1b[0m",
            CheckStatus::Fail => "\x1b[31m[FAIL]\x1b[0m",
        };
        println!("  {} {:<14} {}", icon, check.name, check.detail);
    }

    println!();

    match &report.device_match {
        Some(device) => {
            println!(
                "  \x1b[32mDevice:\x1b[0m {} ({})",
                device.name, device.notes
            );
        }
        None => {
            println!("  \x1b[33mDevice:\x1b[0m Unknown (not in compatibility registry)");
        }
    }

    println!();

    match report.verdict {
        PreflightVerdict::Compatible => {
            println!("  \x1b[1m\x1b[32mVerdict: COMPATIBLE\x1b[0m");
            println!("  This device is fully supported by Nexus Link.");
        }
        PreflightVerdict::NotRecommended => {
            println!("  \x1b[1m\x1b[33mVerdict: NOT RECOMMENDED\x1b[0m");
            println!("  GPU detected but device is not in the compatibility registry.");
            println!("  Registration will proceed but the device is untested.");
            println!("  Run with --force to register anyway.");
        }
        PreflightVerdict::Incompatible => {
            println!("  \x1b[1m\x1b[31mVerdict: INCOMPATIBLE\x1b[0m");
            println!("  No GPU detected. Nexus Link requires NVIDIA GPU hardware.");
            println!("  Linking this device is not supported.");
        }
    }

    println!();
}
