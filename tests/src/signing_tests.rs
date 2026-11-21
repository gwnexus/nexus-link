use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signer, SigningKey};
use nexus_link_core::config::ComposeConfig;
use nexus_link_core::heartbeat::HeartbeatPayload;
use nexus_link_core::signing::{
    KeyLoad, MAX_SIGNING_KEYS, NonceCache, QueueSignatureError, SIGNATURE_WINDOW_SECS,
    SignatureState, VerifyContext, key_fingerprint, load_verifying_keys, queue_message,
    signature_state, verify_queued_command,
};
use nexus_link_core::types::{ComposeCommandItem, ComposeCommandType};
use serde_json::json;

const NODE: &str = "node-1";

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn now() -> DateTime<Utc> {
    "2026-11-17T04:30:00Z".parse().unwrap()
}

/// A queue item signed the way the hub signs it.
fn signed_item(
    key: &SigningKey,
    node: &str,
    id: &str,
    kind: ComposeCommandType,
    payload: &str,
    ts: DateTime<Utc>,
    nonce: &str,
) -> ComposeCommandItem {
    let ts = ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let msg = queue_message(node, id, kind.as_str(), &ts, nonce, payload);
    ComposeCommandItem {
        id: id.into(),
        command_type: kind,
        payload: Some(payload.into()),
        signature: Some(hex::encode(key.sign(msg.as_bytes()).to_bytes())),
        timestamp: Some(ts),
        nonce: Some(nonce.into()),
        ..Default::default()
    }
}

fn put_file(nonce: &str) -> ComposeCommandItem {
    signed_item(
        &signing_key(),
        NODE,
        "cmd-1",
        ComposeCommandType::PutFile,
        r#"{"content":"services: {}"}"#,
        now(),
        nonce,
    )
}

fn verify(
    cmd: &ComposeCommandItem,
    require: bool,
    key: Option<&ed25519_dalek::VerifyingKey>,
    nonces: &NonceCache,
) -> Result<nexus_link_core::signing::VerifiedCommand, QueueSignatureError> {
    verify_queued_command(
        cmd,
        &VerifyContext {
            node_id: NODE,
            require_signatures: require,
            keys: key.map(std::slice::from_ref).unwrap_or(&[]),
            nonces,
            now: now(),
        },
    )
}

#[test]
fn test_signed_command_verifies_and_returns_payload_args() {
    let vk = signing_key().verifying_key();
    let mut cmd = put_file("n1");
    cmd.args = json!({ "content": "ignored when payload is present" });
    let v = verify(&cmd, true, Some(&vk), &NonceCache::new()).unwrap();
    assert!(v.signed);
    assert_eq!(v.args, json!({ "content": "services: {}" }));
}

#[test]
fn test_unsigned_command_rejected_when_enforced() {
    let vk = signing_key().verifying_key();
    let cmd = ComposeCommandItem {
        id: "cmd-1".into(),
        command_type: ComposeCommandType::PutFile,
        args: json!({ "content": "x: 1" }),
        ..Default::default()
    };
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Missing)
    );
}

#[test]
fn test_unsigned_command_accepted_when_not_enforced() {
    let cmd = ComposeCommandItem {
        id: "cmd-1".into(),
        command_type: ComposeCommandType::GetFile,
        args: json!({ "a": 1 }),
        ..Default::default()
    };
    let v = verify(&cmd, false, None, &NonceCache::new()).unwrap();
    assert!(!v.signed);
    assert_eq!(v.args, json!({ "a": 1 }));
}

#[test]
fn test_every_command_type_requires_signature_when_enforced() {
    let vk = signing_key().verifying_key();
    for kind in [
        ComposeCommandType::GetFile,
        ComposeCommandType::PutFile,
        ComposeCommandType::Activate,
        ComposeCommandType::GetLogsSnapshot,
        ComposeCommandType::ValidateYaml,
    ] {
        let cmd = ComposeCommandItem {
            id: "c".into(),
            command_type: kind,
            ..Default::default()
        };
        assert_eq!(
            verify(&cmd, true, Some(&vk), &NonceCache::new()),
            Err(QueueSignatureError::Missing)
        );
    }
}

#[test]
fn test_invalid_signature_rejected_even_when_not_enforced() {
    let vk = signing_key().verifying_key();
    let mut cmd = put_file("n1");
    cmd.payload = Some(r#"{"content":"tampered: true"}"#.into());
    assert_eq!(
        verify(&cmd, false, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Invalid)
    );
}

#[test]
fn test_signature_from_other_key_rejected() {
    let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
    assert_eq!(
        verify(&put_file("n1"), true, Some(&other), &NonceCache::new()),
        Err(QueueSignatureError::Invalid)
    );
}

#[test]
fn test_signature_bound_to_node_id_and_command_id() {
    let vk = signing_key().verifying_key();
    // Signed for another node
    let cmd = signed_item(
        &signing_key(),
        "node-2",
        "cmd-1",
        ComposeCommandType::Activate,
        "{}",
        now(),
        "n1",
    );
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Invalid)
    );
    // Valid signature replayed under another command id
    let mut cmd = put_file("n2");
    cmd.id = "cmd-2".into();
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Invalid)
    );
}

#[test]
fn test_signature_bound_to_command_type() {
    let vk = signing_key().verifying_key();
    let mut cmd = put_file("n1");
    cmd.command_type = ComposeCommandType::Activate;
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Invalid)
    );
}

#[test]
fn test_timestamp_window() {
    let vk = signing_key().verifying_key();
    let edge = now() - Duration::seconds(SIGNATURE_WINDOW_SECS);
    let ok = signed_item(
        &signing_key(),
        NODE,
        "c",
        ComposeCommandType::Activate,
        "{}",
        edge,
        "a",
    );
    assert!(verify(&ok, true, Some(&vk), &NonceCache::new()).is_ok());

    let old = now() - Duration::seconds(SIGNATURE_WINDOW_SECS + 1);
    let late = signed_item(
        &signing_key(),
        NODE,
        "c",
        ComposeCommandType::Activate,
        "{}",
        old,
        "b",
    );
    assert_eq!(
        verify(&late, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::OutsideWindow)
    );

    let future = now() + Duration::seconds(SIGNATURE_WINDOW_SECS + 1);
    let early = signed_item(
        &signing_key(),
        NODE,
        "c",
        ComposeCommandType::Activate,
        "{}",
        future,
        "c",
    );
    assert_eq!(
        verify(&early, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::OutsideWindow)
    );
}

#[test]
fn test_duplicate_nonce_rejected_but_invalid_signature_does_not_burn_nonce() {
    let vk = signing_key().verifying_key();
    let nonces = NonceCache::new();

    let mut bad = put_file("n1");
    bad.signature = Some("00".repeat(64));
    assert_eq!(
        verify(&bad, true, Some(&vk), &nonces),
        Err(QueueSignatureError::Invalid)
    );
    // The same nonce is still usable by the genuine command
    assert!(verify(&put_file("n1"), true, Some(&vk), &nonces).is_ok());
    assert_eq!(
        verify(&put_file("n1"), true, Some(&vk), &nonces),
        Err(QueueSignatureError::DuplicateNonce)
    );
}

#[test]
fn test_incomplete_and_malformed_signature_fields() {
    let vk = signing_key().verifying_key();
    let mut cmd = put_file("n1");
    cmd.nonce = None;
    assert_eq!(
        verify(&cmd, false, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Incomplete)
    );
    let mut cmd = put_file("n1");
    cmd.payload = None;
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Incomplete)
    );
    let mut cmd = put_file("n1");
    cmd.signature = Some("zz".into());
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::Malformed)
    );
    let mut cmd = put_file("n1");
    cmd.timestamp = Some("yesterday".into());
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::BadTimestamp)
    );
}

#[test]
fn test_non_object_payload_rejected() {
    let vk = signing_key().verifying_key();
    let cmd = signed_item(
        &signing_key(),
        NODE,
        "c",
        ComposeCommandType::Activate,
        "[1]",
        now(),
        "n",
    );
    assert_eq!(
        verify(&cmd, true, Some(&vk), &NonceCache::new()),
        Err(QueueSignatureError::BadPayload)
    );
}

#[test]
fn test_no_key_on_node() {
    let cmd = put_file("n1");
    assert_eq!(
        verify(&cmd, true, None, &NonceCache::new()),
        Err(QueueSignatureError::NoKey)
    );
    // Enforcement off: hub may already sign; node without key still works
    let v = verify(&cmd, false, None, &NonceCache::new()).unwrap();
    assert!(!v.signed);
    assert_eq!(v.args, json!({ "content": "services: {}" }));
}

#[test]
fn test_errors_are_reported_with_signature_prefix() {
    for e in [
        QueueSignatureError::Missing,
        QueueSignatureError::Incomplete,
        QueueSignatureError::NoKey,
        QueueSignatureError::BadTimestamp,
        QueueSignatureError::OutsideWindow,
        QueueSignatureError::Malformed,
        QueueSignatureError::Invalid,
        QueueSignatureError::DuplicateNonce,
        QueueSignatureError::BadPayload,
    ] {
        assert!(e.to_string().starts_with("signature: "), "{e}");
    }
}

#[test]
fn test_queue_message_format() {
    let msg = queue_message(
        "node-1",
        "cmd-1",
        "put_file",
        "2026-11-17T04:30:00Z",
        "n1",
        "{}",
    );
    assert_eq!(
        msg,
        "nexus-cmd-v1\nnode-1\ncmd-1\nput_file\n2026-11-17T04:30:00Z\nn1\n\
         44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
}

#[test]
fn test_command_type_wire_names_match_serde() {
    for kind in [
        ComposeCommandType::GetFile,
        ComposeCommandType::PutFile,
        ComposeCommandType::Activate,
        ComposeCommandType::GetLogsSnapshot,
        ComposeCommandType::ValidateYaml,
    ] {
        assert_eq!(serde_json::to_value(&kind).unwrap(), json!(kind.as_str()));
    }
}

#[test]
fn test_signed_queue_item_deserializes() {
    let item: ComposeCommandItem = serde_json::from_value(json!({
        "id": "cmd-1", "type": "put_file",
        "payload": "{\"content\":\"a: 1\"}",
        "signature": "ab", "timestamp": "2026-11-17T04:30:00Z", "nonce": "n"
    }))
    .unwrap();
    assert_eq!(item.command_type, ComposeCommandType::PutFile);
    assert_eq!(item.payload.as_deref(), Some("{\"content\":\"a: 1\"}"));
    // Legacy unsigned item still deserializes
    let legacy: ComposeCommandItem =
        serde_json::from_value(json!({ "id": "c", "type": "activate" })).unwrap();
    assert!(legacy.signature.is_none() && legacy.payload.is_none());
}

#[test]
fn test_signature_state_matrix() {
    let key = KeyLoad::Loaded(vec![signing_key().verifying_key()]);
    assert_eq!(signature_state(true, &key), SignatureState::Enforced);
    assert_eq!(signature_state(false, &key), SignatureState::Off);
    assert_eq!(
        signature_state(true, &KeyLoad::Missing),
        SignatureState::KeyMissing
    );
    assert_eq!(
        signature_state(true, &KeyLoad::Invalid("x")),
        SignatureState::KeyMissing
    );
    assert_eq!(
        signature_state(false, &KeyLoad::Missing),
        SignatureState::Off
    );
}

#[test]
fn test_load_verifying_key_sources() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(signing_key().verifying_key().to_bytes());
    let home = tempfile::tempdir().unwrap();
    let mut config = ComposeConfig::default();

    assert!(matches!(
        load_verifying_keys(&config, &[home.path().to_path_buf()]),
        KeyLoad::Missing
    ));

    config.signing_public_key = Some(b64.clone());
    assert!(matches!(
        load_verifying_keys(&config, &[home.path().to_path_buf()]),
        KeyLoad::Loaded(_)
    ));

    config.signing_public_key = Some("not-a-key".into());
    assert!(matches!(
        load_verifying_keys(&config, &[home.path().to_path_buf()]),
        KeyLoad::Invalid("compose.signing_public_key")
    ));

    // An invalid config key does not fall through to the key file (ADR-0008)
    std::fs::write(home.path().join("signing_key.pub"), format!("{b64}\n")).unwrap();
    assert!(matches!(
        load_verifying_keys(&config, &[home.path().to_path_buf()]),
        KeyLoad::Invalid("compose.signing_public_key")
    ));

    // Without a config key the file is the fallback
    config.signing_public_key = None;
    assert!(matches!(
        load_verifying_keys(&config, &[home.path().to_path_buf()]),
        KeyLoad::Loaded(_)
    ));
}

#[test]
fn test_heartbeat_reports_command_signatures() {
    let body = serde_json::to_value(
        HeartbeatPayload::online(None).with_command_signatures(SignatureState::KeyMissing),
    )
    .unwrap();
    assert_eq!(
        body,
        json!({ "status": "online", "command_signatures": "key_missing" })
    );
    let body = serde_json::to_value(
        HeartbeatPayload::online(None).with_command_signatures(SignatureState::Enforced),
    )
    .unwrap();
    assert_eq!(body["command_signatures"], "enforced");
}

#[test]
fn test_load_verifying_key_search_order() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(signing_key().verifying_key().to_bytes());
    let etc = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let dirs = [etc.path().to_path_buf(), state.path().to_path_buf()];
    let config = ComposeConfig::default();

    // Key only in a later dir (e.g. /var/lib/nexus-link from `setup`)
    std::fs::write(state.path().join("signing_key.pub"), &b64).unwrap();
    assert!(matches!(
        load_verifying_keys(&config, &dirs),
        KeyLoad::Loaded(_)
    ));

    // The first dir holding a key file decides; an invalid one is reported,
    // not silently skipped in favor of the next file
    std::fs::write(etc.path().join("signing_key.pub"), "garbage").unwrap();
    assert!(matches!(
        load_verifying_keys(&config, &dirs),
        KeyLoad::Invalid("signing_key.pub")
    ));
}

// ---------------------------------------------------------------------------
// Per-profile keys and rotation (ADR-0008)
// ---------------------------------------------------------------------------

fn b64(key: &SigningKey) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
}

fn prod_key() -> SigningKey {
    SigningKey::from_bytes(&[11u8; 32])
}

fn staging_key() -> SigningKey {
    SigningKey::from_bytes(&[22u8; 32])
}

fn put_file_signed_by(key: &SigningKey, nonce: &str) -> ComposeCommandItem {
    signed_item(
        key,
        NODE,
        "cmd-1",
        ComposeCommandType::PutFile,
        r#"{"content":"services: {}"}"#,
        now(),
        nonce,
    )
}

fn verify_with(
    cmd: &ComposeCommandItem,
    keys: &[ed25519_dalek::VerifyingKey],
) -> Result<nexus_link_core::signing::VerifiedCommand, QueueSignatureError> {
    verify_queued_command(
        cmd,
        &VerifyContext {
            node_id: NODE,
            require_signatures: true,
            keys,
            nonces: &NonceCache::new(),
            now: now(),
        },
    )
}

#[test]
fn test_config_key_wins_over_shared_key_file() {
    // The shared /etc/nexus-link/signing_key.pub holds the old common key
    let shared = tempfile::tempdir().unwrap();
    std::fs::write(shared.path().join("signing_key.pub"), b64(&signing_key())).unwrap();
    let dirs = [shared.path().to_path_buf()];

    let config = ComposeConfig {
        signing_public_key: Some(b64(&prod_key())),
        ..ComposeConfig::default()
    };
    let load = load_verifying_keys(&config, &dirs);
    assert_eq!(load.keys(), &[prod_key().verifying_key()]);
}

#[test]
fn test_profiles_on_one_node_refuse_each_others_keys() {
    // Both profiles share the key dirs (and a legacy signing_key.pub) but
    // carry their own key in their config.
    let shared = tempfile::tempdir().unwrap();
    std::fs::write(shared.path().join("signing_key.pub"), b64(&signing_key())).unwrap();
    let dirs = [shared.path().to_path_buf()];

    let production = ComposeConfig {
        signing_public_key: Some(b64(&prod_key())),
        ..ComposeConfig::default()
    };
    let staging = ComposeConfig {
        signing_public_key: Some(b64(&staging_key())),
        ..ComposeConfig::default()
    };
    let prod_keys = load_verifying_keys(&production, &dirs);
    let staging_keys = load_verifying_keys(&staging, &dirs);

    // Each profile accepts its own key
    assert!(verify_with(&put_file_signed_by(&prod_key(), "p1"), prod_keys.keys()).is_ok());
    assert!(
        verify_with(
            &put_file_signed_by(&staging_key(), "s1"),
            staging_keys.keys()
        )
        .is_ok()
    );

    // ... and refuses the other profile's key, in both directions
    assert_eq!(
        verify_with(&put_file_signed_by(&staging_key(), "s2"), prod_keys.keys()),
        Err(QueueSignatureError::Invalid)
    );
    assert_eq!(
        verify_with(&put_file_signed_by(&prod_key(), "p2"), staging_keys.keys()),
        Err(QueueSignatureError::Invalid)
    );

    // The old shared key no longer works on either profile
    for keys in [prod_keys.keys(), staging_keys.keys()] {
        assert_eq!(
            verify_with(&put_file_signed_by(&signing_key(), "x"), keys),
            Err(QueueSignatureError::Invalid)
        );
    }
}

#[test]
fn test_rotation_list_accepts_both_keys_and_wins_over_single_key() {
    let new = SigningKey::from_bytes(&[33u8; 32]);
    let config = ComposeConfig {
        signing_public_key: Some(b64(&staging_key())),
        signing_public_keys: Some(vec![b64(&new), b64(&prod_key())]),
        ..ComposeConfig::default()
    };
    let load = load_verifying_keys(&config, &[]);
    assert_eq!(load.keys().len(), 2);

    assert!(verify_with(&put_file_signed_by(&new, "n1"), load.keys()).is_ok());
    assert!(verify_with(&put_file_signed_by(&prod_key(), "o1"), load.keys()).is_ok());
    // signing_public_key is ignored while the list is set
    assert_eq!(
        verify_with(&put_file_signed_by(&staging_key(), "s1"), load.keys()),
        Err(QueueSignatureError::Invalid)
    );
}

#[test]
fn test_invalid_rotation_list_does_not_fall_through() {
    let mut config = ComposeConfig {
        signing_public_key: Some(b64(&prod_key())),
        ..ComposeConfig::default()
    };
    let too_many: Vec<String> = (0..=MAX_SIGNING_KEYS as u8)
        .map(|i| b64(&SigningKey::from_bytes(&[40 + i; 32])))
        .collect();
    for list in [vec![], vec![b64(&prod_key()), "garbage".into()], too_many] {
        config.signing_public_keys = Some(list);
        let load = load_verifying_keys(&config, &[]);
        assert!(matches!(
            load,
            KeyLoad::Invalid("compose.signing_public_keys")
        ));
        assert_eq!(signature_state(true, &load), SignatureState::KeyMissing);
    }
}

#[test]
fn test_key_fingerprint_format() {
    use sha2::{Digest, Sha256};
    let vk = prod_key().verifying_key();
    let fp = key_fingerprint(&vk);
    assert_eq!(fp.len(), 16);
    assert_eq!(fp, hex::encode(Sha256::digest(vk.as_bytes()))[..16]);
    assert_ne!(fp, key_fingerprint(&staging_key().verifying_key()));
}

#[test]
fn test_heartbeat_reports_signing_key_fingerprints() {
    let body = serde_json::to_value(HeartbeatPayload::online(None)).unwrap();
    assert!(body.get("signing_key_fingerprints").is_none());
    let body = serde_json::to_value(
        HeartbeatPayload::online(None)
            .with_signing_key_fingerprints(vec!["e9aab7cd5a2b120e".into()]),
    )
    .unwrap();
    assert_eq!(
        body["signing_key_fingerprints"],
        json!(["e9aab7cd5a2b120e"])
    );
}
