pub mod error;
mod proto;
mod transport;
mod partner;

pub use partner::{S7Partner, PartnerStatus};
pub use error::{Error, Result};
