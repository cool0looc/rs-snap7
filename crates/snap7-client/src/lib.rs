pub mod proto;
pub mod client;
pub mod connection;
pub mod error;
pub mod plus_client;
pub mod plus_connection;
pub mod pool;
pub mod tag;
pub mod tls;
pub mod transport;
pub mod types;
pub mod udp;

#[cfg(feature = "sync")]
pub mod client_sync;

pub use client::{MultiReadItem, MultiWriteItem, S7Client};
pub use error::{Error, Result};
pub use plus_client::S7PlusClient;
pub use plus_connection::{plus_connect, PlusConnection};
pub use pool::{PoolConfig, PooledClient, S7Pool};
pub use proto::ProtoError;
pub use tag::{parse_tag, TagAddress};
pub use tls::{tls_connect, TlsStream};
pub use types::{
    BlockData, BlockInfo, BlockList, BlockListEntry, BlockType, ConnectParams, CpuInfo, CpInfo,
    ModuleEntry, OrderCode, PlcStatus, Protection,
};
pub use types::encrypt_password;
pub use udp::UdpTransport;
