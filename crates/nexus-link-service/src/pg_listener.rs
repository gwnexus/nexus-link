//! PostgreSQL LISTEN/NOTIFY wake-up channel — ADR-0071.
//!
//! Maintains a persistent PG connection subscribed to `node_cmd_<node_id>`.
//! On notification, signals the poller to run an immediate poll cycle.
//! On connection loss, reconnects with exponential backoff while HTTP
//! fallback polling continues independently.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use rustls::ClientConfig;
use tokio::sync::Notify;
use tokio_postgres::AsyncMessage;
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{error, info, warn};

use crate::state::AppState;

/// Maximum reconnect backoff (seconds).
const MAX_BACKOFF_SECS: u64 = 30;

/// Run the PG LISTEN loop. Sends a wake signal on every notification.
/// Never returns — runs forever with automatic reconnection.
pub async fn run(state: Arc<AppState>, wake: Arc<Notify>) {
    let Some(ref database_url) = state.config.api.database_url else {
        return;
    };

    let channel = format!("node_cmd_{}", state.config.node.node_id);
    let mut backoff_secs: u64 = 1;

    loop {
        match connect_and_listen(database_url, &channel, &wake).await {
            Ok(()) => {
                warn!("PG LISTEN connection closed — reconnecting");
                backoff_secs = 1;
            }
            Err(e) => {
                error!(
                    backoff_s = backoff_secs,
                    "PG LISTEN error: {} — reconnecting", e
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

/// Build a rustls TLS connector for PostgreSQL.
fn make_tls_connector() -> MakeRustlsConnect {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    MakeRustlsConnect::new(tls_config)
}

/// Connect, subscribe, and listen until the connection drops.
async fn connect_and_listen(
    database_url: &str,
    channel: &str,
    wake: &Arc<Notify>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tls = make_tls_connector();
    let (client, mut connection) = tokio_postgres::connect(database_url, tls).await?;

    // Extract the notification stream from the connection.
    let stream = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));

    // Subscribe to the node-specific channel
    let listen_stmt = format!("LISTEN \"{}\"", channel.replace('"', "\"\""));
    client.batch_execute(&listen_stmt).await?;
    info!(channel = %channel, "PG LISTEN active");

    // Process the notification stream
    tokio::pin!(stream);
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(AsyncMessage::Notification(n)) => {
                info!(
                    channel = %n.channel(),
                    payload = %n.payload(),
                    "PG NOTIFY received — triggering immediate poll"
                );
                wake.notify_one();
            }
            Ok(AsyncMessage::Notice(notice)) => {
                warn!("PG notice: {}", notice.message());
            }
            Ok(_) => {}
            Err(e) => {
                return Err(e.into());
            }
        }
    }

    Ok(())
}
