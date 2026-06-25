use nexus_link_core::config::Config;

pub async fn execute() -> anyhow::Result<()> {
    let config = Config::load()?;

    println!("Nexus Link Status");
    println!("─────────────────────────────");
    println!("  Node ID:    {}", config.node.node_id);
    println!("  Name:       {}", config.node.name);
    println!("  API:        {}", config.api.base_url);
    println!(
        "  Interval:   {}s push / {}s poll",
        config.agent.push_sec, config.agent.poll_sec
    );
    println!(
        "  Service:    {}:{}",
        config.service.listen_addr, config.service.port
    );
    if !config.node.tags.is_empty() {
        println!("  Tags:       {}", config.node.tags.join(", "));
    }

    // TODO: Check if agent is running (PID file or systemd status)
    // TODO: Show last telemetry push timestamp
    // TODO: Show service health

    Ok(())
}
