use thiserror::Error;

/// The single error type shared across the whole workspace. Keeping it in
/// `hive-core` means every crate speaks the same failure language without
/// depending on each other.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("http error: {0}")]
    Http(String),
    #[error("api error: {0}")]
    Api(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn other(msg: impl Into<String>) -> Self {
        CoreError::Other(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
