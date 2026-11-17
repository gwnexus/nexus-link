//! State of the PostgreSQL LISTEN wake-up channel (hub ADR-0071).
//!
//! The command service holds the listener; the agent sends the heartbeat.
//! Both units of a profile share one systemd `StateDirectory`, so the service
//! writes its state to `$STATE_DIRECTORY/wake_listener` and the agent reports
//! it as `wake_listener` in the heartbeat. Outside systemd (no
//! `STATE_DIRECTORY`) nothing is written and the field is omitted.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// File name inside the state directory.
pub const STATE_FILE: &str = "wake_listener";

/// The service refreshes a `connected` state at least this often (health
/// check interval); an older `connected` file means the service is gone.
pub const CONNECTED_MAX_AGE: Duration = Duration::from_secs(120);

/// Listener state as reported in the heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeListenerState {
    /// LISTEN active and the self-test notification arrived.
    Connected,
    /// Configured but not listening right now (connecting, retrying, or the
    /// self-test failed, e.g. behind a transaction pooler).
    Disconnected,
    /// No `api.database_url`: HTTP polling only.
    Unconfigured,
}

impl WakeListenerState {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeListenerState::Connected => "connected",
            WakeListenerState::Disconnected => "disconnected",
            WakeListenerState::Unconfigured => "unconfigured",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "connected" => Some(WakeListenerState::Connected),
            "disconnected" => Some(WakeListenerState::Disconnected),
            "unconfigured" => Some(WakeListenerState::Unconfigured),
            _ => None,
        }
    }
}

/// `$STATE_DIRECTORY` as set by systemd (first entry if several).
pub fn state_dir_from_env() -> Option<PathBuf> {
    let raw = std::env::var_os("STATE_DIRECTORY")?;
    let first = raw.to_string_lossy().split(':').next()?.to_string();
    (!first.is_empty()).then(|| PathBuf::from(first))
}

/// Write the state atomically (tmp file + rename). Also refreshes the mtime,
/// which `read_state` uses to detect a stopped service.
pub fn write_state(dir: &Path, state: WakeListenerState) -> std::io::Result<()> {
    let path = dir.join(STATE_FILE);
    let tmp = dir.join(format!("{STATE_FILE}.tmp"));
    std::fs::write(&tmp, state.as_str())?;
    std::fs::rename(&tmp, &path)
}

/// Read the state written by the service. `None` when there is no readable
/// state file. A `connected` state older than `CONNECTED_MAX_AGE` (relative
/// to `now`) is reported as `disconnected`.
pub fn read_state(dir: &Path, now: SystemTime) -> Option<WakeListenerState> {
    let path = dir.join(STATE_FILE);
    let state = WakeListenerState::parse(&std::fs::read_to_string(&path).ok()?)?;
    if state == WakeListenerState::Connected {
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        let age = now.duration_since(modified).unwrap_or_default();
        if age > CONNECTED_MAX_AGE {
            return Some(WakeListenerState::Disconnected);
        }
    }
    Some(state)
}
