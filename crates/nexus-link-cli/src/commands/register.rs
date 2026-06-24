use nexus_link_core::config::{ApiConfig, ComposeConfig, Config, NodeConfig, ServiceConfig, dirs_home};
use nexus_link_core::preflight::{self, PreflightVerdict};
use nexus_link_core::types::RegisterRequest;
use tracing::info;

pub async fn execute(
    api_url: String,
    token: String,
    cmd_token: Option<String>,
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
            PreflightVerdict::Compatible => {}
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

    // Validate node token format
    if !nexus_link_core::token::validate_token_format(&token) {
        anyhow::bail!("Invalid token format. Expected: nxs_node_<...>");
    }

    // Validate cmd_token format if provided
    if let Some(ref ct) = cmd_token {
        if !nexus_link_core::token::validate_cmd_token_format(ct) {
            anyhow::bail!("Invalid cmd-token format. Expected: nxs_cmd_<...>");
        }
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

    // Prefer the cmd_token from the backend response; fall back to the CLI flag.
    let resolved_cmd_token = register_resp.cmd_token.clone().or(cmd_token);

    // Prefer signing_public_key from the backend response.
    let signing_public_key = register_resp.signing_public_key.clone();

    // Write signing_key.pub to ~/.nexus-link/ if the backend provided one
    if let Some(ref pubkey_b64) = signing_public_key {
        let home = dirs_home();
        std::fs::create_dir_all(&home)?;
        let key_path = home.join("signing_key.pub");
        std::fs::write(&key_path, pubkey_b64)?;
        println!("  Signing key: {}", key_path.display());
    }

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
            push_interval_secs: 10,
        },
        service: ServiceConfig::default(),
        compose: ComposeConfig {
            cmd_token: resolved_cmd_token,
            signing_public_key,
            ..ComposeConfig::default()
        },
    };

    config.save()?;

    println!("Node registered successfully!");
    println!("  Node ID: {}", register_resp.node_id);
    println!("  Name:    {}", node_name);
    println!(
        "  Config:  {}",
        nexus_link_core::config::default_config_path().display()
    );

    // Remind the operator if the C&C channel is not yet configured
    if config.compose.cmd_token.is_none() {
        println!();
        println!("  Note: C&C channel not configured (--cmd-token not provided).");
        println!("  Compose management API will return 403 until a cmd token is set:");
        println!("    nexus-link config set compose.cmd_token <nxs_cmd_*>");
    }

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
