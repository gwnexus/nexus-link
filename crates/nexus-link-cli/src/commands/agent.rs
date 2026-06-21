use tracing::info;

pub async fn start() -> anyhow::Result<()> {
    info!("Starting nexus-link agent and service...");

    // TODO: Fork/daemonize nexus-link-agent and nexus-link-service
    // For v0.1.0: use systemd unit files on the target machine
    // Alternative: spawn both as child processes and write PID files

    println!("nexus-link agent started");
    println!("  Telemetry push: active");
    println!("  Command service: listening on :8443");

    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    info!("Stopping nexus-link agent and service...");

    // TODO: Read PID files and send SIGTERM
    // Or: systemctl stop nexus-link-agent nexus-link-service

    println!("nexus-link agent stopped");

    Ok(())
}

pub async fn logs(tail: u32) -> anyhow::Result<()> {
    // TODO: Read from journald or log files
    println!("Showing last {} log lines...", tail);
    println!(
        "(not yet implemented -- use: journalctl -u nexus-link-agent -n {})",
        tail
    );

    Ok(())
}
