use std::time::{Duration, SystemTime};

use nexus_link_core::heartbeat::HeartbeatPayload;
use nexus_link_core::wake::{CONNECTED_MAX_AGE, WakeListenerState, read_state, write_state};
use serde_json::json;

#[test]
fn test_wake_state_roundtrip_and_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_state(dir.path(), SystemTime::now()), None);
    for s in [
        WakeListenerState::Connected,
        WakeListenerState::Disconnected,
        WakeListenerState::Unconfigured,
    ] {
        write_state(dir.path(), s).unwrap();
        assert_eq!(read_state(dir.path(), SystemTime::now()), Some(s));
    }
}

#[test]
fn test_stale_connected_state_reads_as_disconnected() {
    let dir = tempfile::tempdir().unwrap();
    write_state(dir.path(), WakeListenerState::Connected).unwrap();
    let later = SystemTime::now() + CONNECTED_MAX_AGE + Duration::from_secs(5);
    assert_eq!(
        read_state(dir.path(), later),
        Some(WakeListenerState::Disconnected)
    );
    // `unconfigured` is written once and never refreshed: it never goes stale.
    write_state(dir.path(), WakeListenerState::Unconfigured).unwrap();
    assert_eq!(
        read_state(dir.path(), later),
        Some(WakeListenerState::Unconfigured)
    );
}

#[test]
fn test_garbage_state_file_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("wake_listener"), "maybe").unwrap();
    assert_eq!(read_state(dir.path(), SystemTime::now()), None);
}

#[test]
fn test_heartbeat_wake_listener_field() {
    let none =
        serde_json::to_value(HeartbeatPayload::online(None).with_wake_listener(None)).unwrap();
    assert!(none.get("wake_listener").is_none());
    let body = serde_json::to_value(
        HeartbeatPayload::online(None).with_wake_listener(Some(WakeListenerState::Connected)),
    )
    .unwrap();
    assert_eq!(
        body,
        json!({ "status": "online", "wake_listener": "connected" })
    );
}
