use nexus_link_core::config::{self, Config};

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
    println!("  push_interval    = {}s", config.api.push_interval_secs);
    println!();
    println!("  [service]");
    println!("  listen_addr      = {}", config.service.listen_addr);
    println!("  port             = {}", config.service.port);

    Ok(())
}

pub async fn set(key: String, value: String) -> anyhow::Result<()> {
    let mut config = Config::load()?;

    match key.as_str() {
        "api.base_url" | "api_url" => {
            config.api.base_url = value.clone();
            println!("  api.base_url = {}", value);
        }
        "api.push_interval_secs" | "push_interval" | "interval" => {
            let secs: u64 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid value: expected integer (seconds)"))?;
            if secs < 5 {
                anyhow::bail!("Push interval must be at least 5 seconds");
            }
            config.api.push_interval_secs = secs;
            println!("  api.push_interval_secs = {}", secs);
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
        _ => {
            anyhow::bail!(
                "Unknown config key: '{}'\n\nAvailable keys:\n  \
                 api_url, push_interval, listen_addr, port, name, tags",
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
        "api.push_interval_secs" | "push_interval" | "interval" => {
            config.api.push_interval_secs.to_string()
        }
        "service.listen_addr" | "listen_addr" => config.service.listen_addr,
        "service.port" | "port" => config.service.port.to_string(),
        "node.name" | "name" => config.node.name,
        "node.node_id" | "node_id" => config.node.node_id,
        "node.tags" | "tags" => config.node.tags.join(","),
        "node.token" | "token" => config.node.token,
        _ => {
            anyhow::bail!(
                "Unknown config key: '{}'\n\nAvailable keys:\n  \
                 api_url, push_interval, listen_addr, port, name, node_id, tags, token",
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
