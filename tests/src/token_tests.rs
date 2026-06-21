use nexus_link_core::token;

#[test]
fn test_validate_token_format_valid() {
    assert!(token::validate_token_format("nxs_node_abc123xyz"));
    assert!(token::validate_token_format("nxs_node_a"));
    assert!(token::validate_token_format(
        "nxs_node_dGhpcyBpcyBhIHRlc3QgdG9rZW4"
    ));
}

#[test]
fn test_validate_token_format_invalid() {
    assert!(!token::validate_token_format("nxs_node_"));
    assert!(!token::validate_token_format("nxs_pat_abc123"));
    assert!(!token::validate_token_format("invalid_token"));
    assert!(!token::validate_token_format(""));
    assert!(!token::validate_token_format("nxs_nod_close_but_no"));
}

#[test]
fn test_generate_token_format() {
    let token = token::generate_token();
    assert!(token.starts_with("nxs_node_"));
    assert!(token.len() > 9); // prefix + at least some content
    assert!(token::validate_token_format(&token));
}

#[test]
fn test_generate_token_uniqueness() {
    let t1 = token::generate_token();
    let t2 = token::generate_token();
    assert_ne!(t1, t2, "Generated tokens should be unique");
}

#[test]
fn test_hash_token_deterministic() {
    let token = "nxs_node_test_token_123";
    let h1 = token::hash_token(token);
    let h2 = token::hash_token(token);
    assert_eq!(h1, h2, "Hashing same input should be deterministic");
}

#[test]
fn test_hash_token_different_inputs() {
    let h1 = token::hash_token("nxs_node_aaa");
    let h2 = token::hash_token("nxs_node_bbb");
    assert_ne!(h1, h2, "Different tokens should produce different hashes");
}

#[test]
fn test_hash_token_hex_format() {
    let hash = token::hash_token("nxs_node_test");
    assert_eq!(hash.len(), 64, "SHA-256 hex should be 64 chars");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "Hash should be valid hex"
    );
}

#[test]
fn test_verify_token_correct() {
    let token = "nxs_node_my_secret_token";
    let hash = token::hash_token(token);
    assert!(token::verify_token(token, &hash));
}

#[test]
fn test_verify_token_wrong() {
    let token = "nxs_node_my_secret_token";
    let hash = token::hash_token("nxs_node_different_token");
    assert!(!token::verify_token(token, &hash));
}
