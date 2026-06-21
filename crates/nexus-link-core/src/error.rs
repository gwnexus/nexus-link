use thiserror::Error;

#[derive(Debug, Error)]
pub enum NexusLinkError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("API request failed: {0}")]
    Api(String),

    #[error("Docker error: {0}")]
    Docker(String),

    #[error("Token validation failed: {0}")]
    TokenValidation(String),

    #[error("Service error: {0}")]
    Service(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
