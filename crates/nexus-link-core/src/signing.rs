//! Ed25519 verification of signed C&C commands (ADR-0004, hub ADR-0051).
//!
//! The hub signs commands with its private key; the node verifies them with
//! the public keys of the active profile (ADR-0008, `load_verifying_keys`).
//!
//! This module covers the command queue (`GET .../compose/commands/pending`).
//! A signed queue item carries the arguments as the exact JSON string the hub
//! signed (`payload`), so no JSON canonicalization is needed across languages:
//!
//! ```text
//! nexus-cmd-v1
//! <node_id>
//! <command id>
//! <type>
//! <timestamp>
//! <nonce>
//! <sha256 hex of the payload bytes>
//! ```
//!
//! Lines are joined with `\n`, without a trailing newline. Binding node id
//! and command id prevents replaying a signed command to another node or
//! under another id; timestamp window and nonce cache prevent replays within
//! the same node.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::ComposeConfig;
use crate::types::ComposeCommandItem;

/// Domain separator and version of the queue message format.
pub const QUEUE_MESSAGE_VERSION: &str = "nexus-cmd-v1";

/// Accepted clock skew between hub signature and node, in both directions.
pub const SIGNATURE_WINDOW_SECS: i64 = 5 * 60;

/// How long a nonce is remembered; slightly longer than the window.
const NONCE_TTL_SECS: i64 = SIGNATURE_WINDOW_SECS + 60;

/// File name of the verifying key in the node's home directory.
pub const SIGNING_KEY_FILE: &str = "signing_key.pub";

// ---------------------------------------------------------------------------
// Verifying key
// ---------------------------------------------------------------------------

/// Decode a base64url (no padding) Ed25519 verifying key.
pub fn decode_verifying_key(b64: &str) -> Option<VerifyingKey> {
    let bytes = URL_SAFE_NO_PAD.decode(b64.trim()).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}

/// Maximum number of keys in `compose.signing_public_keys` (ADR-0008): the
/// current key and the next one during a hub key rotation.
pub const MAX_SIGNING_KEYS: usize = 2;

/// Where the verifying keys came from, or why none could be loaded.
#[derive(Debug)]
pub enum KeyLoad {
    /// One or more keys; a signature is valid if any of them verifies it.
    Loaded(Vec<VerifyingKey>),
    /// No key source is set.
    Missing,
    /// A source is set but does not hold valid keys; names the source.
    Invalid(&'static str),
}

impl KeyLoad {
    /// The loaded keys; empty when missing or invalid.
    pub fn keys(&self) -> &[VerifyingKey] {
        match self {
            KeyLoad::Loaded(k) => k,
            _ => &[],
        }
    }
}

/// Load the verifying keys of one profile (ADR-0008). The first source that
/// is set decides:
///
/// 1. `compose.signing_public_keys` (1 to `MAX_SIGNING_KEYS` keys),
/// 2. `compose.signing_public_key`,
/// 3. the first `signing_key.pub` found in `key_dirs` (legacy fallback, see
///    `config::signing_key_dirs`; these dirs are shared between profiles).
///
/// A source that is set but invalid does not fall through to a lower one;
/// the caller logs it and enforcement rejects every command.
pub fn load_verifying_keys(compose: &ComposeConfig, key_dirs: &[PathBuf]) -> KeyLoad {
    if let Some(ref list) = compose.signing_public_keys {
        let keys: Option<Vec<VerifyingKey>> =
            list.iter().map(|b64| decode_verifying_key(b64)).collect();
        return match keys {
            Some(k) if !k.is_empty() && k.len() <= MAX_SIGNING_KEYS => KeyLoad::Loaded(k),
            _ => KeyLoad::Invalid("compose.signing_public_keys"),
        };
    }

    if let Some(ref b64) = compose.signing_public_key {
        return match decode_verifying_key(b64) {
            Some(key) => KeyLoad::Loaded(vec![key]),
            None => KeyLoad::Invalid("compose.signing_public_key"),
        };
    }

    match key_dirs
        .iter()
        .find_map(|dir| std::fs::read_to_string(dir.join(SIGNING_KEY_FILE)).ok())
    {
        Some(content) => match decode_verifying_key(&content) {
            Some(key) => KeyLoad::Loaded(vec![key]),
            None => KeyLoad::Invalid(SIGNING_KEY_FILE),
        },
        None => KeyLoad::Missing,
    }
}

/// Whether any of `keys` verifies `signature` over `message`.
pub fn verify_any(keys: &[VerifyingKey], message: &[u8], signature: &Signature) -> bool {
    keys.iter().any(|k| k.verify(message, signature).is_ok())
}

/// Fingerprint of a verifying key as the hub shows it: SHA-256 of the raw
/// 32 bytes, first 16 hex characters.
pub fn key_fingerprint(key: &VerifyingKey) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))[..16].to_string()
}

// ---------------------------------------------------------------------------
// Enforcement state (reported in the heartbeat)
// ---------------------------------------------------------------------------

/// Command signature enforcement as reported in the heartbeat
/// (`command_signatures`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureState {
    /// Enforcement on and a verifying key is loaded.
    Enforced,
    /// Enforcement off; unsigned commands are executed.
    Off,
    /// Enforcement on but no valid verifying key: every command is rejected.
    KeyMissing,
}

/// Enforcement state from the config and the key load result.
pub fn signature_state(require_signatures: bool, key: &KeyLoad) -> SignatureState {
    match (require_signatures, key.keys().is_empty()) {
        (false, _) => SignatureState::Off,
        (true, false) => SignatureState::Enforced,
        (true, true) => SignatureState::KeyMissing,
    }
}

// ---------------------------------------------------------------------------
// Nonce replay cache
// ---------------------------------------------------------------------------

/// Remembers nonces of accepted signatures until they leave the window.
/// Shared by the HTTP command path and the queue.
#[derive(Debug, Default)]
pub struct NonceCache {
    seen: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl NonceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `nonce`; returns `false` if it was already seen (replay).
    pub fn check_and_insert(&self, nonce: &str, now: DateTime<Utc>) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, expiry| *expiry > now);
        if seen.contains_key(nonce) {
            return false;
        }
        seen.insert(nonce.to_string(), now + Duration::seconds(NONCE_TTL_SECS));
        true
    }
}

// ---------------------------------------------------------------------------
// Queue command verification
// ---------------------------------------------------------------------------

/// Why a queued command was rejected. `Display` is reported to the hub as the
/// command error and always starts with `signature:`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueueSignatureError {
    #[error("signature: missing (node enforces signed commands)")]
    Missing,
    #[error("signature: incomplete (signature, timestamp, nonce and payload are all required)")]
    Incomplete,
    #[error("signature: no verifying key on node")]
    NoKey,
    #[error("signature: invalid timestamp format")]
    BadTimestamp,
    #[error("signature: timestamp outside window")]
    OutsideWindow,
    #[error("signature: malformed signature")]
    Malformed,
    #[error("signature: invalid")]
    Invalid,
    #[error("signature: duplicate nonce")]
    DuplicateNonce,
    #[error("signature: payload is not a JSON object")]
    BadPayload,
}

/// Outcome of a successful check: the arguments to execute with.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedCommand {
    pub args: serde_json::Value,
    /// Whether a signature was present and verified (false for an unsigned
    /// command accepted because enforcement is off).
    pub signed: bool,
}

/// Inputs that do not come from the command itself.
pub struct VerifyContext<'a> {
    pub node_id: &'a str,
    pub require_signatures: bool,
    /// Verifying keys of the profile; empty when none is loaded.
    pub keys: &'a [VerifyingKey],
    pub nonces: &'a NonceCache,
    pub now: DateTime<Utc>,
}

/// The exact message the hub signs for a queued command.
pub fn queue_message(
    node_id: &str,
    command_id: &str,
    command_type: &str,
    timestamp: &str,
    nonce: &str,
    payload: &str,
) -> String {
    let payload_hash = hex::encode(Sha256::digest(payload.as_bytes()));
    [
        QUEUE_MESSAGE_VERSION,
        node_id,
        command_id,
        command_type,
        timestamp,
        nonce,
        &payload_hash,
    ]
    .join("\n")
}

/// Verify a queued command and return the arguments to execute with.
///
/// - No signature fields: rejected when enforcement is on, otherwise the
///   command runs unsigned (arguments from `payload` if present, else `args`).
/// - Signature fields present: always verified, also with enforcement off,
///   so hub signing can be tested before enforcement. Exception: a node
///   without a verifying key and with enforcement off runs the command and
///   reports it as unsigned.
/// - The nonce is recorded only after the signature verified, so invalid
///   signatures cannot burn nonces.
pub fn verify_queued_command(
    cmd: &ComposeCommandItem,
    ctx: &VerifyContext<'_>,
) -> Result<VerifiedCommand, QueueSignatureError> {
    let fields = (
        cmd.signature.as_deref(),
        cmd.timestamp.as_deref(),
        cmd.nonce.as_deref(),
    );

    let (sig_hex, timestamp, nonce) = match fields {
        (None, None, None) => {
            if ctx.require_signatures {
                return Err(QueueSignatureError::Missing);
            }
            let args = match cmd.payload.as_deref() {
                Some(p) => parse_payload(p)?,
                None => cmd.args.clone(),
            };
            return Ok(VerifiedCommand {
                args,
                signed: false,
            });
        }
        (Some(s), Some(t), Some(n)) => (s, t, n),
        _ => return Err(QueueSignatureError::Incomplete),
    };
    let payload = cmd
        .payload
        .as_deref()
        .ok_or(QueueSignatureError::Incomplete)?;

    if ctx.keys.is_empty() {
        if ctx.require_signatures {
            return Err(QueueSignatureError::NoKey);
        }
        return Ok(VerifiedCommand {
            args: parse_payload(payload)?,
            signed: false,
        });
    }

    let ts: DateTime<Utc> = timestamp
        .parse()
        .map_err(|_| QueueSignatureError::BadTimestamp)?;
    if (ctx.now - ts).num_seconds().abs() > SIGNATURE_WINDOW_SECS {
        return Err(QueueSignatureError::OutsideWindow);
    }

    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(QueueSignatureError::Malformed)?;
    let signature = Signature::from_bytes(&sig_bytes);

    let message = queue_message(
        ctx.node_id,
        &cmd.id,
        cmd.command_type.as_str(),
        timestamp,
        nonce,
        payload,
    );
    if !verify_any(ctx.keys, message.as_bytes(), &signature) {
        return Err(QueueSignatureError::Invalid);
    }

    if !ctx.nonces.check_and_insert(nonce, ctx.now) {
        return Err(QueueSignatureError::DuplicateNonce);
    }

    Ok(VerifiedCommand {
        args: parse_payload(payload)?,
        signed: true,
    })
}

fn parse_payload(payload: &str) -> Result<serde_json::Value, QueueSignatureError> {
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(v @ serde_json::Value::Object(_)) => Ok(v),
        _ => Err(QueueSignatureError::BadPayload),
    }
}
