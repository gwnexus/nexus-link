//! Hub-published `nexus-cmd-v1` signing vectors (ADR-0007, Dispatch c2696a25).
//!
//! The fixture is vendored unchanged from the hub at `HUB_COMMIT` (see
//! `tests/fixtures/hub/README.md`). The signatures come from the public
//! RFC 8032 section 7.1 TEST 1 key pair, which no environment uses.

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, VerifyingKey};
use nexus_link_core::signing::{
    NonceCache, QueueSignatureError, VerifyContext, queue_message, verify_queued_command,
};
use nexus_link_core::types::{ComposeCommandItem, ComposeCommandType};
use serde_json::Value;

/// Hub commit the fixture was taken from.
const HUB_COMMIT: &str = "d5228b4bfeca26c975662b78a68b72c064cb2a6c";

const VECTORS: &str = include_str!("../fixtures/hub/nexus-cmd-v1.vectors.json");

/// Types in the pinned vectors the node does not execute (reported to the
/// hub in Dispatch c2696a25). Their message and signature are still checked.
const NOT_NODE_TYPES: &[&str] = &["compose_ps", "read_file"];

struct Vector {
    name: String,
    node_id: String,
    command_id: String,
    command_type: String,
    timestamp: String,
    nonce: String,
    payload: String,
    payload_sha256: String,
    message: String,
    signature: String,
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("vector field `{key}` missing or not a string"))
}

fn load() -> (VerifyingKey, Vec<Vector>) {
    let doc: Value = serde_json::from_str(VECTORS).expect("fixture is valid JSON");
    assert_eq!(
        doc["contract"], "nexus-cmd-v1",
        "fixture from hub {HUB_COMMIT}"
    );
    assert_eq!(doc["version"], 1);

    let pk: [u8; 32] = hex::decode(str_field(&doc["test_key"], "public_key_hex"))
        .expect("public key hex")
        .try_into()
        .expect("32-byte public key");
    let key = VerifyingKey::from_bytes(&pk).expect("valid Ed25519 public key");

    let vectors = doc["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .map(|v| {
            let input = &v["input"];
            Vector {
                name: str_field(v, "name").into(),
                node_id: str_field(input, "nodeId").into(),
                command_id: str_field(input, "commandId").into(),
                command_type: str_field(input, "type").into(),
                timestamp: str_field(input, "timestamp").into(),
                nonce: str_field(input, "nonce").into(),
                payload: str_field(input, "payload").into(),
                payload_sha256: str_field(v, "payload_sha256").into(),
                message: str_field(v, "message").into(),
                signature: str_field(v, "signature").into(),
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(vectors.len(), 3, "pinned fixture has three vectors");
    (key, vectors)
}

fn message_for(v: &Vector, node_id: &str) -> String {
    queue_message(
        node_id,
        &v.command_id,
        &v.command_type,
        &v.timestamp,
        &v.nonce,
        &v.payload,
    )
}

fn signature(v: &Vector) -> Signature {
    let bytes: [u8; 64] = hex::decode(&v.signature)
        .expect("signature hex")
        .try_into()
        .expect("64-byte signature");
    Signature::from_bytes(&bytes)
}

/// The node's queue item for a vector, if the node knows the command type.
fn queue_item(v: &Vector) -> Option<ComposeCommandItem> {
    let command_type: ComposeCommandType =
        serde_json::from_value(Value::String(v.command_type.clone())).ok()?;
    Some(ComposeCommandItem {
        id: v.command_id.clone(),
        command_type,
        payload: Some(v.payload.clone()),
        signature: Some(v.signature.clone()),
        timestamp: Some(v.timestamp.clone()),
        nonce: Some(v.nonce.clone()),
        ..Default::default()
    })
}

fn ctx<'a>(
    v: &Vector,
    node_id: &'a str,
    key: &'a VerifyingKey,
    nonces: &'a NonceCache,
) -> VerifyContext<'a> {
    // Clock stub: "now" is the signing time, so the window check passes.
    let now: DateTime<Utc> = v.timestamp.parse().expect("RFC 3339 timestamp");
    VerifyContext {
        node_id,
        require_signatures: true,
        key: Some(key),
        nonces,
        now,
    }
}

#[test]
fn test_hub_vectors_message_matches_node_construction() {
    let (_, vectors) = load();
    for v in &vectors {
        let message = message_for(v, &v.node_id);
        assert_eq!(
            message, v.message,
            "message differs for vector `{}`",
            v.name
        );
        // The last line is the node's own sha256 of the payload bytes.
        let hash = message.rsplit('\n').next().unwrap();
        assert_eq!(
            hash, v.payload_sha256,
            "payload hash differs for `{}`",
            v.name
        );
    }
}

#[test]
fn test_hub_vectors_signatures_verify_with_test_key() {
    let (key, vectors) = load();
    for v in &vectors {
        key.verify_strict(message_for(v, &v.node_id).as_bytes(), &signature(v))
            .unwrap_or_else(|e| panic!("signature of `{}` does not verify: {e}", v.name));
    }
}

#[test]
fn test_hub_vectors_pass_the_queue_verification_path() {
    let (key, vectors) = load();
    let mut checked = 0;
    for v in &vectors {
        let Some(item) = queue_item(v) else {
            assert!(
                NOT_NODE_TYPES.contains(&v.command_type.as_str()),
                "vector `{}` has unexpected type `{}`",
                v.name,
                v.command_type
            );
            continue;
        };
        let nonces = NonceCache::new();
        let verified = verify_queued_command(&item, &ctx(v, &v.node_id, &key, &nonces))
            .unwrap_or_else(|e| panic!("vector `{}` rejected: {e}", v.name));
        assert!(
            verified.signed,
            "vector `{}` not reported as signed",
            v.name
        );
        let payload: Value = serde_json::from_str(&v.payload).unwrap();
        assert_eq!(verified.args, payload, "args differ for `{}`", v.name);
        checked += 1;
    }
    assert!(checked >= 1, "no vector uses a node command type");
}

#[test]
fn test_hub_vectors_fail_for_another_node_id() {
    let (key, vectors) = load();
    let other = "00000000-0000-4000-8000-000000000000";
    for v in &vectors {
        assert_ne!(other, v.node_id);
        assert!(
            key.verify_strict(message_for(v, other).as_bytes(), &signature(v))
                .is_err(),
            "vector `{}` verified for a different node_id",
            v.name
        );
        if let Some(item) = queue_item(v) {
            let nonces = NonceCache::new();
            assert_eq!(
                verify_queued_command(&item, &ctx(v, other, &key, &nonces)).unwrap_err(),
                QueueSignatureError::Invalid,
                "vector `{}` accepted on a different node",
                v.name
            );
        }
    }
}
