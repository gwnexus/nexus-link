use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

const TOKEN_PREFIX: &str = "nxs_node_";
const CMD_TOKEN_PREFIX: &str = "nxs_cmd_";

// ---------------------------------------------------------------------------
// Node token (nxs_node_*) — identity / telemetry
// ---------------------------------------------------------------------------

/// Generate a new node token: nxs_node_<base64url(32 random bytes)>
pub fn generate_token() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    format!("{}{}", TOKEN_PREFIX, URL_SAFE_NO_PAD.encode(bytes))
}

/// Validate node token format (starts with nxs_node_)
pub fn validate_token_format(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX) && token.len() > TOKEN_PREFIX.len()
}

// ---------------------------------------------------------------------------
// C&C token (nxs_cmd_*) — command & control channel
// ---------------------------------------------------------------------------

/// Generate a new C&C command token: nxs_cmd_<base64url(32 random bytes)>
pub fn generate_cmd_token() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    format!("{}{}", CMD_TOKEN_PREFIX, URL_SAFE_NO_PAD.encode(bytes))
}

/// Validate C&C token format (starts with nxs_cmd_)
pub fn validate_cmd_token_format(token: &str) -> bool {
    token.starts_with(CMD_TOKEN_PREFIX) && token.len() > CMD_TOKEN_PREFIX.len()
}

// ---------------------------------------------------------------------------
// Shared — hashing and verification
// ---------------------------------------------------------------------------

/// Hash a token with SHA-256 for server-side storage.
/// Works for both nxs_node_* and nxs_cmd_* tokens.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Verify a token against a stored hash.
///
/// Note: We compare SHA-256 hex digests using standard equality. This is NOT
/// constant-time, but timing attacks on hash comparisons are not practically
/// exploitable when the input is a 64-char hex string (network jitter dominates).
/// The security property comes from the pre-image resistance of SHA-256, not
/// from constant-time comparison of the hashes.
pub fn verify_token(token: &str, stored_hash: &str) -> bool {
    hash_token(token) == stored_hash
}

// We use hex encoding for hashes
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
