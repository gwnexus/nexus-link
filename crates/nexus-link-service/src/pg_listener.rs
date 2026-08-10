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
                    "PG LISTEN error: {:?} — reconnecting", e
                );
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

/// Build a rustls TLS connector for PostgreSQL.
/// Supabase Pooler uses a private CA (Supabase Root 2021 CA) not present in
/// public root stores. With `sslmode=require` the connection is encrypted but
/// we must accept the server's certificate without full chain validation.
/// This matches the behavior of libpq with `sslmode=require` (encrypt only,
/// no identity verification).
fn make_tls_connector() -> MakeRustlsConnect {
    // Ensure a CryptoProvider is installed (ring via feature flag).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let tls_config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();

    MakeRustlsConnect::new(tls_config)
}

/// Certificate verifier that accepts any server certificate.
/// Equivalent to PostgreSQL `sslmode=require` — connection is encrypted
/// but server identity is not verified. Acceptable because:
/// 1. The database_url contains the known Supabase pooler hostname
/// 2. DNS resolution is trusted (local resolver)
/// 3. The connection carries a scoped read-only listener credential
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
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
