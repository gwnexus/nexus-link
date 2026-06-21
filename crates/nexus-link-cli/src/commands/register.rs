use nexus_link_core::config::{ApiConfig, Config, NodeConfig, ServiceConfig};
use nexus_link_core::preflight::{self, PreflightVerdict};
use nexus_link_core::types::RegisterRequest;
use tracing::info;

pub async fn execute(
    api_url: String,
    token: String,
    name: Option<String>,
    tags: Vec<String>,
    skip_preflight: bool,
    force: bool,
) -> anyhow::Result<()> {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let node_name = name.unwrap_or_else(|| hostname.clone());

    // Run preflight check unless explicitly skipped
    if !skip_preflight {
        info!("Running device preflight check...");
        let report = preflight::run_preflight();
        preflight::print_report(&report);

        match report.verdict {
            PreflightVerdict::Compatible => {
                // All good, continue
            }
            PreflightVerdict::NotRecommended => {
                if !force {
                    anyhow::bail!(
                        "Device is not in the compatibility registry. \
                         Use --force to register anyway, or --skip-preflight to bypass checks."
                    );
                }
                println!("  Proceeding with registration (--force)...");
                println!();
            }
            PreflightVerdict::Incompatible => {
                if !force {
                    anyhow::bail!(
                        "Device is incompatible (no GPU detected). \
                         Nexus Link requires NVIDIA GPU hardware."
                    );
                }
                println!("  WARNING: Forcing registration on incompatible device...");
                println!();
            }
        }
    }

    info!("Registering node '{}' with Nexus at {}", node_name, api_url);

    // Validate token format
    if !nexus_link_core::token::validate_token_format(&token) {
        anyhow::bail!("Invalid token format. Expected: nxs_node_<...>");
    }

    let client = reqwest::Client::new();
    let register_req = RegisterRequest {
        name: node_name.clone(),
        private_ip: None, // TODO: detect local IP
        tags: tags.clone(),
        description: Some(format!("Registered via nexus-link CLI from {}", hostname)),
    };

    let resp = client
        .post(format!("{}/api/nodes/register", api_url))
        .bearer_auth(&token)
        .json(&register_req)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Registration failed ({}): {}", status, body);
    }

    let register_resp: nexus_link_core::types::RegisterResponse = resp.json().await?;

    // Save config locally
    let config = Config {
        node: NodeConfig {
            node_id: register_resp.node_id.clone(),
            name: node_name.clone(),
            token: register_resp.token.clone(),
            tags,
        },
        api: ApiConfig {
            base_url: api_url,
            push_interval_secs: 30,
        },
        service: ServiceConfig::default(),
    };

    config.save()?;

    println!("Node registered successfully!");
    println!("  Node ID: {}", register_resp.node_id);
    println!("  Name:    {}", node_name);
    println!(
        "  Config:  {}",
        nexus_link_core::config::default_config_path().display()
    );

    Ok(())
}

// Minimal hostname helper
mod hostname {
    use std::ffi::OsString;

    pub fn get() -> Result<OsString, std::io::Error> {
        #[cfg(unix)]
        {
            use std::process::Command;
            let output = Command::new("hostname").output()?;
            Ok(OsString::from(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            ))
        }
        #[cfg(not(unix))]
        {
            Ok(OsString::from("unknown"))
        }
    }
}
