//! PostgreSQL LISTEN/NOTIFY wake-up channel (hub ADR-0071).
//!
//! Holds `LISTEN node_cmd_<node_id>` on `api.database_url`. The hub runs
//! `pg_notify('node_cmd_' || node_id, command_id)` on every queued command;
//! a notification triggers an immediate poll. HTTP polling keeps running
//! independently, so the listener only lowers latency.
//!
//! - The connection is driven by its own task. (Before 0.21.5 the `LISTEN`
//!   was awaited before anything polled the connection, so it hung forever:
//!   no error, no reconnect, no wake-up.)
//! - After `LISTEN` the node notifies its own channel and waits for that
//!   probe. A DSN that cannot deliver notifications (e.g. a transaction
//!   pooler) fails this self-test instead of silently never waking up.
//! - A `SELECT 1` health check and TCP keepalives detect dead connections;
//!   every failure reconnects with exponential backoff.
//! - The state is logged at start and on every change (never the DSN) and
//!   written for the heartbeat (`nexus_link_core::wake`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use nexus_link_core::wake::{self, WakeListenerState};
use rustls::ClientConfig;
use tokio::sync::{Notify, mpsc};
use tokio::time::timeout;
use tokio_postgres::{AsyncMessage, Notification};
use tokio_postgres_rustls::MakeRustlsConnect;
use tracing::{error, info, warn};

use crate::state::AppState;

/// Maximum reconnect backoff.
const MAX_BACKOFF_SECS: u64 = 30;
/// Retry delay after a failed self-test (notifications are not delivered;
/// retrying quickly will not help until the DSN changes).
const SELF_TEST_RETRY_SECS: u64 = 300;
/// Health check interval; also refreshes the `connected` state file, so it
/// must stay well below `wake::CONNECTED_MAX_AGE`.
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
/// Timeout for connect, `LISTEN`, the self-test and each health check.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Payload of the self-test notification; never triggers a poll.
const PROBE_PAYLOAD: &str = "nexus-link-listener-probe";
/// Supabase transaction pooler port; it does not deliver notifications.
const TRANSACTION_POOLER_PORT: u16 = 6543;

/// Channel the hub notifies: `node_cmd_<node_id>`, node id lowercase as in
/// `ai_nodes.id`.
pub fn channel_name(node_id: &str) -> String {
    format!("node_cmd_{}", node_id.trim().to_lowercase())
}

/// Logs state changes and keeps the heartbeat state file current.
pub struct StateReporter {
    dir: Option<PathBuf>,
    last: Option<WakeListenerState>,
}

impl StateReporter {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir, last: None }
    }

    pub fn set(&mut self, state: WakeListenerState) {
        if self.last != Some(state) {
            info!(state = state.as_str(), "PG LISTEN wake-up state");
            self.last = Some(state);
        }
        if let Some(dir) = &self.dir
            && let Err(e) = wake::write_state(dir, state)
        {
            warn!(error = %e, "Cannot write wake listener state file");
        }
    }
}

/// Run the listener for the service. Returns only when the listener cannot
/// run at all (not configured, unparsable DSN).
pub async fn run(state: Arc<AppState>, wake: Arc<Notify>) {
    let mut reporter = StateReporter::new(wake::state_dir_from_env());
    let Some(database_url) = state.config.api.database_url.as_deref() else {
        reporter.set(WakeListenerState::Unconfigured);
        info!("PG LISTEN not configured (no api.database_url); HTTP polling only");
        return;
    };
    let config = match database_url.parse::<tokio_postgres::Config>() {
        Ok(c) => c,
        Err(e) => {
            reporter.set(WakeListenerState::Disconnected);
            error!(error = %e, "PG LISTEN: api.database_url is not a valid connection string; HTTP polling only");
            return;
        }
    };
    listen_loop(
        config,
        &channel_name(&state.config.node.node_id),
        &wake,
        &mut reporter,
    )
    .await;
}

/// Reconnect loop. Never returns.
pub async fn listen_loop(
    mut config: tokio_postgres::Config,
    channel: &str,
    wake: &Arc<Notify>,
    reporter: &mut StateReporter,
) {
    config
        .keepalives(true)
        .keepalives_idle(Duration::from_secs(30))
        .connect_timeout(STEP_TIMEOUT);
    if config.get_ports().contains(&TRANSACTION_POOLER_PORT) {
        warn!(
            port = TRANSACTION_POOLER_PORT,
            "api.database_url uses the Supabase transaction pooler port, which does not \
             deliver LISTEN notifications; the hub should hand out the session pooler or a \
             direct connection"
        );
    }
    info!(channel = %channel, "PG LISTEN starting");

    let mut backoff_secs: u64 = 1;
    loop {
        reporter.set(WakeListenerState::Disconnected);
        let delay = match listen_once(&config, channel, wake, reporter).await {
            ListenEnd::Closed => {
                warn!("PG LISTEN connection closed; reconnecting");
                backoff_secs = 1;
                backoff_secs
            }
            ListenEnd::SelfTestFailed => {
                error!(
                    retry_s = SELF_TEST_RETRY_SECS,
                    "PG LISTEN self-test failed: LISTEN succeeded but the probe notification \
                     never arrived (a transaction pooler drops notifications); HTTP polling \
                     continues"
                );
                SELF_TEST_RETRY_SECS
            }
            ListenEnd::Failed {
                step,
                error,
                was_connected,
            } => {
                if was_connected {
                    backoff_secs = 1;
                }
                warn!(step, error = %error, backoff_s = backoff_secs, "PG LISTEN failed; retrying");
                backoff_secs
            }
        };
        reporter.set(WakeListenerState::Disconnected);
        tokio::time::sleep(Duration::from_secs(delay)).await;
        if delay == backoff_secs {
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
        }
    }
}

/// How one connection ended.
enum ListenEnd {
    /// The server closed the connection after it was listening.
    Closed,
    /// LISTEN worked but notifications are not delivered.
    SelfTestFailed,
    /// A step failed or timed out.
    Failed {
        step: &'static str,
        error: String,
        was_connected: bool,
    },
}

fn failed(step: &'static str, error: impl std::fmt::Display, was_connected: bool) -> ListenEnd {
    ListenEnd::Failed {
        step,
        error: error.to_string(),
        was_connected,
    }
}

/// Aborts the connection driver when the listener returns.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Quote `s` as an SQL string literal.
fn sql_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// One connection: connect, LISTEN, self-test, then forward notifications
/// until the connection ends or a health check fails.
async fn listen_once(
    config: &tokio_postgres::Config,
    channel: &str,
    wake: &Arc<Notify>,
    reporter: &mut StateReporter,
) -> ListenEnd {
    let (client, mut connection) =
        match timeout(STEP_TIMEOUT, config.connect(make_tls_connector())).await {
            Err(_) => return failed("connect", "timeout", false),
            Ok(Err(e)) => return failed("connect", e, false),
            Ok(Ok(pair)) => pair,
        };

    // Drive the connection in its own task; requests on `client` only
    // complete while the connection is polled.
    let (tx, mut rx) = mpsc::unbounded_channel::<Result<Notification, tokio_postgres::Error>>();
    let driver = tokio::spawn(async move {
        let mut messages = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(msg) = messages.next().await {
            let forward = match msg {
                Ok(AsyncMessage::Notification(n)) => Ok(n),
                Ok(AsyncMessage::Notice(notice)) => {
                    warn!("PG notice: {}", notice.message());
                    continue;
                }
                Ok(_) => continue,
                Err(e) => Err(e),
            };
            let stop = forward.is_err();
            if tx.send(forward).is_err() || stop {
                break;
            }
        }
    });
    let _driver = AbortOnDrop(driver);

    let listen = format!("LISTEN \"{}\"", channel.replace('"', "\"\""));
    match timeout(STEP_TIMEOUT, client.batch_execute(&listen)).await {
        Err(_) => return failed("listen", "timeout", false),
        Ok(Err(e)) => return failed("listen", e, false),
        Ok(Ok(())) => {}
    }

    // Self-test: our own notification must come back on this connection.
    let probe = format!(
        "SELECT pg_notify({}, {})",
        sql_literal(channel),
        sql_literal(PROBE_PAYLOAD)
    );
    match timeout(STEP_TIMEOUT, client.batch_execute(&probe)).await {
        Err(_) => return failed("self-test", "timeout", false),
        Ok(Err(e)) => return failed("self-test", e, false),
        Ok(Ok(())) => {}
    }
    let deadline = tokio::time::Instant::now() + STEP_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) => return ListenEnd::SelfTestFailed,
            Ok(None) => return failed("self-test", "connection closed", false),
            Ok(Some(Err(e))) => return failed("self-test", e, false),
            Ok(Some(Ok(n))) if n.payload() == PROBE_PAYLOAD => break,
            // A real command raced the probe: wake the poller and keep waiting.
            Ok(Some(Ok(_))) => wake.notify_one(),
        }
    }

    reporter.set(WakeListenerState::Connected);
    info!(channel = %channel, "PG LISTEN active (self-test passed)");

    let mut health = tokio::time::interval(HEALTH_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    health.tick().await; // first tick is immediate
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                None => return ListenEnd::Closed,
                Some(Err(e)) => return failed("notification stream", e, true),
                Some(Ok(n)) if n.payload() == PROBE_PAYLOAD => {}
                Some(Ok(n)) => {
                    info!(command_id = %n.payload(), "PG NOTIFY received; polling now");
                    wake.notify_one();
                }
            },
            _ = health.tick() => {
                match timeout(STEP_TIMEOUT, client.simple_query("SELECT 1")).await {
                    Err(_) => return failed("health check", "timeout", true),
                    Ok(Err(e)) => return failed("health check", e, true),
                    // Refreshes the state file for the heartbeat.
                    Ok(Ok(_)) => reporter.set(WakeListenerState::Connected),
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_lowercase_node_id() {
        assert_eq!(
            channel_name(" 6F1C2A0E-3B4D-4E5F-8A9B-0C1D2E3F4A5B "),
            "node_cmd_6f1c2a0e-3b4d-4e5f-8a9b-0c1d2e3f4a5b"
        );
    }

    #[test]
    fn sql_literal_escapes_quotes() {
        assert_eq!(sql_literal("a'b"), "'a''b'");
    }

    #[test]
    fn reporter_writes_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = StateReporter::new(Some(dir.path().to_path_buf()));
        r.set(WakeListenerState::Disconnected);
        r.set(WakeListenerState::Connected);
        assert_eq!(
            wake::read_state(dir.path(), std::time::SystemTime::now()),
            Some(WakeListenerState::Connected)
        );
    }

    // ── Live tests against a real PostgreSQL ────────────────────────────
    // NEXUS_LINK_TEST_PG_URL=postgres://user:pw@host:port/db \
    //   cargo test -p nexus-link-service -- --ignored pg_listener

    fn test_url() -> String {
        std::env::var("NEXUS_LINK_TEST_PG_URL").expect("set NEXUS_LINK_TEST_PG_URL")
    }

    async fn side_client() -> tokio_postgres::Client {
        let (c, conn) = tokio_postgres::connect(&test_url(), tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(conn);
        c
    }

    async fn wait_for_state(dir: &std::path::Path, want: WakeListenerState) {
        for _ in 0..100 {
            if wake::read_state(dir, std::time::SystemTime::now()) == Some(want) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("listener did not reach state {want:?}");
    }

    fn start(channel: &str, dir: &std::path::Path) -> (Arc<Notify>, tokio::task::JoinHandle<()>) {
        let config: tokio_postgres::Config = test_url().parse().unwrap();
        let wake = Arc::new(Notify::new());
        let (w, ch, d) = (Arc::clone(&wake), channel.to_string(), dir.to_path_buf());
        let handle = tokio::spawn(async move {
            let mut reporter = StateReporter::new(Some(d));
            listen_loop(config, &ch, &w, &mut reporter).await;
        });
        (wake, handle)
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL server (NEXUS_LINK_TEST_PG_URL)"]
    async fn live_notify_wakes_the_poller() {
        let dir = tempfile::tempdir().unwrap();
        let (wake, handle) = start("node_cmd_live-a", dir.path());
        wait_for_state(dir.path(), WakeListenerState::Connected).await;

        side_client()
            .await
            .batch_execute("SELECT pg_notify('node_cmd_live-a', 'cmd-1')")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), wake.notified())
            .await
            .expect("no wake-up within 3 s after NOTIFY");
        handle.abort();
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL server (NEXUS_LINK_TEST_PG_URL)"]
    async fn live_reconnects_after_the_backend_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let (wake, handle) = start("node_cmd_live-b", dir.path());
        wait_for_state(dir.path(), WakeListenerState::Connected).await;

        let side = side_client().await;
        side.batch_execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE query LIKE '%node_cmd_live-b%' AND pid <> pg_backend_pid()",
        )
        .await
        .unwrap();
        wait_for_state(dir.path(), WakeListenerState::Disconnected).await;
        wait_for_state(dir.path(), WakeListenerState::Connected).await;

        side.batch_execute("SELECT pg_notify('node_cmd_live-b', 'cmd-2')")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), wake.notified())
            .await
            .expect("no wake-up after reconnect");
        handle.abort();
    }

    #[tokio::test]
    #[ignore = "needs a PostgreSQL server (NEXUS_LINK_TEST_PG_URL)"]
    async fn live_probe_does_not_wake_the_poller() {
        let dir = tempfile::tempdir().unwrap();
        let (wake, handle) = start("node_cmd_live-c", dir.path());
        wait_for_state(dir.path(), WakeListenerState::Connected).await;
        // The self-test probe must not have left a wake permit behind.
        assert!(
            tokio::time::timeout(Duration::from_millis(500), wake.notified())
                .await
                .is_err()
        );
        handle.abort();
    }
}
