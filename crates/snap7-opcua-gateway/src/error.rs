use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("PLC error: {0}")]
    Plc(#[from] snap7_client::Error),

    #[error("OPC-UA error: {0}")]
    OpcUa(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("invalid tag '{tag}': {reason}")]
    InvalidTag { tag: String, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;
