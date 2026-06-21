use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest, Sha256};

const TOKEN_PREFIX: &str = "nxs_node_";

/// Generate a new node token: nxs_node_<base64url(32 random bytes)>
pub fn generate_token() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    format!("{}{}", TOKEN_PREFIX, URL_SAFE_NO_PAD.encode(bytes))
}

/// Hash a token with SHA-256 for server-side storage
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Validate token format (starts with nxs_node_)
pub fn validate_token_format(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX) && token.len() > TOKEN_PREFIX.len()
}

/// Verify a token against a stored hash
pub fn verify_token(token: &str, stored_hash: &str) -> bool {
    hash_token(token) == stored_hash
}

// We use hex encoding for hashes -- add dep
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
