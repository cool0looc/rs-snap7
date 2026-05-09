use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid PDU: {0}")]
    InvalidPdu(&'static str),
    #[error("remote refused BSend (err code {0:#06x})")]
    SendRefused(u16),
    #[error("receive timed out")]
    RecvTimeout,
    #[error("partner not connected")]
    NotConnected,
}

pub type Result<T> = std::result::Result<T, Error>;
