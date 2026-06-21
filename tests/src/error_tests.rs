use nexus_link_core::error::NexusLinkError;

#[test]
fn test_error_display_config() {
    let err = NexusLinkError::Config("missing field".to_string());
    assert_eq!(err.to_string(), "Configuration error: missing field");
}

#[test]
fn test_error_display_auth() {
    let err = NexusLinkError::Auth("invalid token".to_string());
    assert_eq!(err.to_string(), "Authentication failed: invalid token");
}

#[test]
fn test_error_display_api() {
    let err = NexusLinkError::Api("timeout".to_string());
    assert_eq!(err.to_string(), "API request failed: timeout");
}

#[test]
fn test_error_display_docker() {
    let err = NexusLinkError::Docker("daemon unreachable".to_string());
    assert_eq!(err.to_string(), "Docker error: daemon unreachable");
}

#[test]
fn test_error_display_token() {
    let err = NexusLinkError::TokenValidation("bad format".to_string());
    assert_eq!(err.to_string(), "Token validation failed: bad format");
}

#[test]
fn test_error_display_service() {
    let err = NexusLinkError::Service("port in use".to_string());
    assert_eq!(err.to_string(), "Service error: port in use");
}

#[test]
fn test_error_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err: NexusLinkError = io_err.into();
    assert!(err.to_string().contains("file not found"));
}
