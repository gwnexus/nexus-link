use nexus_link_core::config::{self, Config, TokenEntry, dirs_home};

pub async fn show() -> anyhow::Result<()> {
    let config_path = config::default_config_path();

    if !config_path.exists() {
        println!("No config found at {}", config_path.display());
        println!("Run 'nexus-link register' first.");
        return Ok(());
    }

    let config = Config::load()?;

    println!("Nexus Link Configuration");
    println!("========================");
    println!();
    println!("  Config file: {}", config_path.display());
    println!();
    println!("  [node]");
    println!("  node_id          = {}", config.node.node_id);
    println!("  name             = {}", config.node.name);
    println!(
        "  token            = {}...",
        &config.node.token[..20.min(config.node.token.len())]
    );
    if !config.node.tags.is_empty() {
        println!("  tags             = {}", config.node.tags.join(", "));
    }
    println!();
    println!("  [api]");
    println!("  base_url         = {}", config.api.base_url);
    println!();
    println!("  [api.tokens]");
    match &config.api.tokens.telemetry {
        Some(t) => println!(
            "  telemetry        = {{ token = {}..., scope = \"{}\" }}",
            &t.token[..20.min(t.token.len())],
            t.scope
        ),
        None => println!("  telemetry        = (not configured)"),
    }
    match &config.api.tokens.command {
        Some(t) => println!(
            "  command          = {{ token = {}..., scope = \"{}\" }}",
            &t.token[..20.min(t.token.len())],
            t.scope
        ),
        None => println!("  command          = (not configured)"),
    }
    println!();
    println!("  [agent]");
    println!("  push_sec         = {}", config.agent.push_sec);
    println!("  poll_sec         = {}", config.agent.poll_sec);
    println!();
    println!("  [service]");
    println!("  listen_addr      = {}", config.service.listen_addr);
    println!("  port             = {}", config.service.port);
    println!();
    println!("  [compose]");
    println!("  dir              = {}", config.compose.dir.display());
    println!(
        "  extra_extensions = {}",
        config.compose.extra_extensions.join(", ")
    );
    match &config.compose.cmd_token {
        Some(t) => println!("  cmd_token        = {}...", &t[..20.min(t.len())]),
        None => println!("  cmd_token        = (not configured — C&C channel disabled)"),
    }
    match &config.compose.signing_public_key {
        Some(k) => println!("  signing_key      = {}...", &k[..20.min(k.len())]),
        None => println!("  signing_key      = (not configured — signatures disabled)"),
    }
    println!(
        "  require_signatures = {}",
        config.compose.require_signatures
    );

    Ok(())
}

pub async fn set(key: String, value: String) -> anyhow::Result<()> {
    let mut config = Config::load()?;

    match key.as_str() {
        "api.base_url" | "api_url" => {
            config.api.base_url = value.clone();
            println!("  api.base_url = {}", value);
        }
        "agent.push_sec" | "api.push_interval_secs" | "push_interval" | "interval" => {
            let secs: u64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid value: expected integer (seconds)"))?;
            if secs < 1 {
                anyhow::bail!("Push interval must be at least 1 second");
            }
            config.agent.push_sec = secs;
            println!("  agent.push_sec = {}", secs);
        }
        "agent.poll_sec" | "poll_sec" => {
            let secs: u64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid value: expected integer (seconds)"))?;
            if secs < 1 {
                anyhow::bail!("Poll interval must be at least 1 second");
            }
            config.agent.poll_sec = secs;
            println!("  agent.poll_sec = {}", secs);
        }
        "api.tokens.telemetry" | "telemetry_token" => {
            config.api.tokens.telemetry = if value.is_empty() {
                None
            } else {
                Some(TokenEntry {
                    token: value.clone(),
                    scope: "read".to_string(),
                })
            };
            // Keep node.token in sync
            if let Some(ref t) = config.api.tokens.telemetry {
                config.node.token = t.token.clone();
            }
            println!("  api.tokens.telemetry updated");
        }
        "api.tokens.command" | "command_token" => {
            config.api.tokens.command = if value.is_empty() {
                None
            } else {
                Some(TokenEntry {
                    token: value.clone(),
                    scope: "read_write".to_string(),
                })
            };
            // Keep compose.cmd_token in sync for backward compat
            config.compose.cmd_token = config.api.tokens.command.as_ref().map(|t| t.token.clone());
            println!("  api.tokens.command updated");
        }
        "service.listen_addr" | "listen_addr" => {
            config.service.listen_addr = value.clone();
            println!("  service.listen_addr = {}", value);
        }
        "service.port" | "port" => {
            let port: u16 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid value: expected port number (1-65535)"))?;
            config.service.port = port;
            println!("  service.port = {}", port);
        }
        "node.name" | "name" => {
            config.node.name = value.clone();
            println!("  node.name = {}", value);
        }
        "node.tags" | "tags" => {
            config.node.tags = value.split(',').map(|s| s.trim().to_string()).collect();
            println!("  node.tags = {:?}", config.node.tags);
        }
        "compose.dir" | "compose_dir" => {
            config.compose.dir = std::path::PathBuf::from(&value);
            println!("  compose.dir = {}", value);
        }
        "compose.cmd_token" | "cmd_token" => {
            if !value.is_empty() && !nexus_link_core::token::validate_cmd_token_format(&value) {
                anyhow::bail!("Invalid cmd_token format. Expected: nxs_cmd_<...>");
            }
            config.compose.cmd_token = if value.is_empty() {
                None
            } else {
                Some(value.clone())
            };
            println!("  compose.cmd_token = {}...", &value[..20.min(value.len())]);
        }
        "compose.require_signatures" | "require_signatures" => {
            let v: bool = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Expected true or false"))?;
            config.compose.require_signatures = v;
            println!("  compose.require_signatures = {}", v);
        }
        "compose.signing_public_key" | "signing_public_key" => {
            let key_path = dirs_home().join("signing_key.pub");
            if value.is_empty() {
                config.compose.signing_public_key = None;
                let _ = std::fs::remove_file(&key_path);
                println!("  compose.signing_public_key cleared");
            } else {
                // Write to signing_key.pub so the service picks it up at next start
                if let Some(parent) = key_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&key_path, &value)?;
                config.compose.signing_public_key = Some(value.clone());
                println!(
                    "  compose.signing_public_key = {}...",
                    &value[..20.min(value.len())]
                );
                println!("  signing_key.pub written to: {}", key_path.display());
            }
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: '{}'\n\nAvailable keys:\n  \
                 api_url, agent.push_sec, agent.poll_sec,\n  \
                 api.tokens.telemetry, api.tokens.command,\n  \
                 listen_addr, port, name, tags,\n  \
                 compose_dir, compose.cmd_token, compose.signing_public_key,\n  \
                 compose.require_signatures",
                key
            );
        }
    }

    config.save()?;
    println!("  Config saved.");

    Ok(())
}

pub async fn get(key: String) -> anyhow::Result<()> {
    let config = Config::load()?;

    let value = match key.as_str() {
        "api.base_url" | "api_url" => config.api.base_url,
        "agent.push_sec" | "api.push_interval_secs" | "push_interval" | "interval" => {
            config.agent.push_sec.to_string()
        }
        "agent.poll_sec" | "poll_sec" => config.agent.poll_sec.to_string(),
        "api.tokens.telemetry" | "telemetry_token" => config
            .api
            .tokens
            .telemetry
            .as_ref()
            .map(|t| t.token.clone())
            .unwrap_or_else(|| "(not configured)".to_string()),
        "api.tokens.command" | "command_token" => config
            .api
            .tokens
            .command
            .as_ref()
            .map(|t| t.token.clone())
            .unwrap_or_else(|| "(not configured)".to_string()),
        "service.listen_addr" | "listen_addr" => config.service.listen_addr,
        "service.port" | "port" => config.service.port.to_string(),
        "node.name" | "name" => config.node.name,
        "node.node_id" | "node_id" => config.node.node_id,
        "node.tags" | "tags" => config.node.tags.join(","),
        "node.token" | "token" => config.node.token,
        "compose.dir" | "compose_dir" => config.compose.dir.to_string_lossy().to_string(),
        "compose.cmd_token" | "cmd_token" => config
            .compose
            .cmd_token
            .unwrap_or_else(|| "(not configured)".to_string()),
        "compose.require_signatures" | "require_signatures" => {
            config.compose.require_signatures.to_string()
        }
        "compose.signing_public_key" | "signing_public_key" => config
            .compose
            .signing_public_key
            .unwrap_or_else(|| "(not configured)".to_string()),
        _ => {
            anyhow::bail!(
                "Unknown config key: '{}'\n\nAvailable keys:\n  \
                 api_url, agent.push_sec, agent.poll_sec,\n  \
                 api.tokens.telemetry, api.tokens.command,\n  \
                 listen_addr, port, name, node_id, tags, token,\n  \
                 compose_dir, compose.cmd_token, compose.signing_public_key,\n  \
                 compose.require_signatures",
                key
            );
        }
    };

    println!("{}", value);
    Ok(())
}

pub async fn path() -> anyhow::Result<()> {
    println!("{}", config::default_config_path().display());
    Ok(())
}
