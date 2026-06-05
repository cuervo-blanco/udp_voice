use thiserror::Error;

pub type Result<T> = std::result::Result<T, LanChatError>;

#[derive(Debug, Error)]
pub enum LanChatError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("mDNS error: {0}")]
    Mdns(#[from] mdns_sd::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
