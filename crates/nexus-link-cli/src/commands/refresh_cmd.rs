use nexus_link_core::config::Config;

/// Apply a rotated C&C command token to the local config.
///
/// The backend's `POST /api/nodes/:id/rotate-cmd` invalidates the old token
/// immediately.  The operator copies the new `nxs_cmd_*` token from the
/// dashboard and runs:
///
///   nexus-link refresh-cmd --cmd-token <nxs_cmd_*>
///
/// nexus-link-service reads `compose.cmd_token` from config on every request
/// (no in-memory cache), so the new token takes effect on the next request
/// without restarting any service.
pub async fn execute(cmd_token: String) -> anyhow::Result<()> {
    // 1. Validate nxs_cmd_* prefix
    if !nexus_link_core::token::validate_cmd_token_format(&cmd_token) {
        anyhow::bail!(
            "Invalid cmd-token format. Expected: nxs_cmd_<...>\n\
             Copy the new token from the Nexus dashboard (Hardware → Node → Rotate C&C Token)."
        );
    }

    // 2. Load current config
    let mut config = Config::load()?;

    // 3. Check for no-op
    if config.compose.cmd_token.as_deref() == Some(cmd_token.as_str()) {
        println!("C&C token is already set to the provided value. Nothing to do.");
        return Ok(());
    }

    let node_name = &config.node.name;
    let prefix = cmd_token[..cmd_token.len().min(16)].to_owned();

    println!("Updating C&C token for node '{}'...", node_name);

    // 4. Write new token — no service restart required
    config.compose.cmd_token = Some(cmd_token);
    config.save()?;

    println!("  C&C token updated. compose.cmd_token prefix: {}...", prefix);
    println!("  Effective immediately — no service restart required.");

    Ok(())
}
