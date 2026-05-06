use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort { need: usize, have: usize },
    #[error("invalid magic byte: expected {expected:#04x}, got {got:#04x}")]
    InvalidMagic { expected: u8, got: u8 },
    #[error("unsupported PDU type: {0:#04x}")]
    UnsupportedPduType(u8),
    #[error("unsupported function code: {0:#04x}")]
    UnsupportedFunction(u8),
    #[error("encoding failed: {0}")]
    EncodingFailed(String),
    #[error("unsupported area code: {0:#04x}")]
    UnsupportedArea(u8),
    #[error("unsupported transport size: {0:#04x}")]
    UnsupportedTransportSize(u8),
    #[error("unsupported S7CommPlus version: {0:#04x}")]
    InvalidVersion(u8),
    #[error("S7CommPlus integrity check failed")]
    IntegrityFailure,
}
