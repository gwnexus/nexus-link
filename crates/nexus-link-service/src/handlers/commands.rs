use std::sync::Arc;

use axum::{Json, extract::State};
use nexus_link_core::types::{CommandResponse, NodeCommand};
use tracing::{error, info};

use crate::state::SharedState;

/// Execute a command received from the Nexus backend
pub async fn execute_command(
    State(state): State<SharedState>,
    Json(command): Json<NodeCommand>,
) -> Json<CommandResponse> {
    info!(?command, "Received command");

    let result = match command {
        NodeCommand::ComposeRestart(payload) => handle_compose_restart(&state, payload).await,
        NodeCommand::ComposeLogs(payload) => handle_compose_logs(&state, payload).await,
        NodeCommand::ConfigExchange(payload) => handle_config_exchange(&state, payload).await,
    };

    match result {
        Ok(resp) => Json(resp),
        Err(e) => {
            error!("Command execution failed: {}", e);
            Json(CommandResponse {
                success: false,
                message: format!("Command failed: {}", e),
                data: None,
            })
        }
    }
}

async fn handle_compose_restart(
    _state: &Arc<crate::state::AppState>,
    payload: nexus_link_core::types::ComposeRestartPayload,
) -> anyhow::Result<CommandResponse> {
    // TODO: Execute docker compose restart via bollard or shell
    let target = payload.service.as_deref().unwrap_or("all services");
    info!("Restarting compose: {}", target);

    Ok(CommandResponse {
        success: true,
        message: format!("Restart initiated for: {}", target),
        data: None,
    })
}

async fn handle_compose_logs(
    _state: &Arc<crate::state::AppState>,
    payload: nexus_link_core::types::ComposeLogsPayload,
) -> anyhow::Result<CommandResponse> {
    // TODO: Fetch container logs via bollard
    info!(
        "Fetching logs for: {} (tail: {})",
        payload.service, payload.tail
    );

    Ok(CommandResponse {
        success: true,
        message: format!("Logs for: {}", payload.service),
        data: Some(serde_json::json!({ "lines": [] })),
    })
}

async fn handle_config_exchange(
    _state: &Arc<crate::state::AppState>,
    payload: nexus_link_core::types::ConfigExchangePayload,
) -> anyhow::Result<CommandResponse> {
    // TODO: Implement config read/write
    info!("Config exchange for key: {}", payload.key);

    Ok(CommandResponse {
        success: true,
        message: format!("Config key: {}", payload.key),
        data: None,
    })
}
