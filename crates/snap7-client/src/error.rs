use crate::proto::ProtoError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("protocol error: {0}")]
    Proto(#[from] ProtoError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PLC error: code={code:#06x} ({message})")]
    PlcError { code: u32, message: String },

    #[error("connection timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("PDU negotiation failed")]
    NegotiationFailed,

    #[error("connection refused or PLC not responding")]
    ConnectionRefused,

    #[error("unexpected response PDU type")]
    UnexpectedResponse,
}

pub type Result<T> = std::result::Result<T, Error>;
