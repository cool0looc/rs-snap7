//! snap7-opcua-gateway: Bridges Siemens S7 PLCs to OPC-UA clients
//!
//! This crate provides an OPC-UA server that polls data from S7 PLCs and exposes
//! it as OPC-UA variable nodes. Tag values are periodically read from the PLC and
//! updated in the OPC-UA address space, making them readable (and optionally writable)
//! over the OPC-UA protocol.

pub mod config;
pub mod error;
pub mod poller;
pub mod registry;
pub mod server;

pub use config::{GatewayConfig, TagSpec};
pub use error::{Error, Result};
pub use poller::PlcPoller;
pub use registry::TagRegistry;
pub use server::Gateway;

// Re-export commonly used types
pub use snap7_client::tag::TagAddress;
