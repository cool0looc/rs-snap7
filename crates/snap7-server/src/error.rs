use snap7_client::proto::ProtoError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Proto(#[from] ProtoError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("negotiation failed")]
    NegotiationFailed,
}
