//! Gateway server - OPC-UA server that bridges S7 PLCs to OPC-UA clients
//! 
//! This module implements an OPC-UA server that bridges S7 PLCs to OPC-UA clients.
//! It supports:
//! - **Read**: Periodic polling of PLC data blocks and exposing them as OPC-UA variables
//! - **Write**: Forwarding OPC-UA client write requests to PLC data blocks
//! - **Subscriptions**: OPC-UA clients can subscribe to variable changes (automatic notifications
//!   when PLC data changes via polling interval)
//!
//! # Subscription Support
//! 
//! The gateway automatically supports OPC-UA subscriptions. Clients can:
//! 1. Create a subscription to receive notifications
//! 2. Create monitored items for specific variables
//! 3. Receive automatic notifications when variable values change
//! 
//! Note: Subscription notifications are sent at the polling interval. For real-time updates,
//! reduce the `poll_interval_ms` in the configuration.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use opcua::server::address_space::{AccessLevel, VariableBuilder};
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::simple_node_manager;
use opcua::server::{Server, ServerBuilder, ServerHandle, ANONYMOUS_USER_TOKEN_ID};
use opcua::types::{
    DataTypeId, LocalizedText, MessageSecurityMode, NodeId, ObjectId,
    Variant,
};
use tokio::sync::mpsc;

use crate::config::GatewayConfig;
use crate::error::{Error, Result};
use crate::poller::{PlcPoller, WriteCommand};
use crate::registry::TagRegistry;
use crate::registry::TagEntry;

/// OPC-UA Gateway - bridges S7 PLCs to OPC-UA clients
pub struct Gateway {
    server: Server,
    handle: ServerHandle,
    registry: Arc<TagRegistry>,
    plc_addr: SocketAddr,
    poll_interval: Duration,
    endpoint_url: String,
    /// Kept alive so write callbacks can clone it; dropping this closes the channel.
    #[allow(dead_code)]
    write_tx: mpsc::Sender<WriteCommand>,
    write_rx: mpsc::Receiver<WriteCommand>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl Gateway {
    /// Create a new Gateway from configuration
    pub async fn new(config: GatewayConfig) -> Result<Self> {
        if config.tags.is_empty() {
            return Err(Error::Config("tags list must not be empty".to_string()));
        }
        if config.poll_interval_ms == 0 {
            return Err(Error::Config(
                "poll_interval_ms must be greater than zero".to_string(),
            ));
        }

        let registry = Arc::new(
            TagRegistry::from_specs(&config.tags).map_err(|e| Error::Config(e.to_string()))?,
        );

        let plc_addr: SocketAddr = config
            .plc_addr
            .parse()
            .map_err(|_| Error::Config(format!("invalid plc_addr: {}", config.plc_addr)))?;

        // Create namespace metadata
        let namespace_meta = NamespaceMetadata {
            namespace_uri: "urn:snap7-opcua-gateway:tags".to_string(),
            namespace_index: 2,
            ..Default::default()
        };

        // Build OPC-UA server with no security (anonymous, no certificates)
        // Parse the OPC endpoint URL to extract just the path component
        let opc_url = url::Url::parse(&config.opc_endpoint)
            .map_err(|e| Error::Config(format!("invalid opc_endpoint URL: {}", e)))?;
        let endpoint_path = opc_url.path();

        let server = ServerBuilder::new()
            .application_name("snap7-opcua-gateway")
            .application_uri("urn:snap7-opcua-gateway")
            .host("0.0.0.0")
            .port(4840)
            .discovery_urls(vec![config.opc_endpoint.clone()])
            .with_node_manager(simple_node_manager(namespace_meta, "snap7-gateway"))
            .add_endpoint(
                "none",
                (
                    endpoint_path,
                    opcua::crypto::SecurityPolicy::None,
                    MessageSecurityMode::None,
                    &[ANONYMOUS_USER_TOKEN_ID] as &[&str],
                ),
            )
            .build()
            .map_err(|e| Error::OpcUa(e.to_string()))?;

        // Server builder returns (Server, ServerHandle)
        let (server, handle) = server;

        let endpoint_url = config.opc_endpoint.clone();

        let (write_tx, write_rx) = mpsc::channel::<WriteCommand>(100);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Ok(Self {
            server,
            handle,
            registry,
            plc_addr,
            poll_interval: Duration::from_millis(config.poll_interval_ms),
            endpoint_url,
            write_tx,
            write_rx,
            shutdown_tx,
        })
    }

    /// Get the endpoint URL
    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    /// Run the gateway server
    pub async fn run(self) -> Result<()> {
        let address_space = self.handle
            .node_managers()
            .get_of_type::<opcua::server::node_manager::memory::SimpleNodeManager>()
            .ok_or_else(|| Error::OpcUa("Failed to get node manager".to_string()))?
            .address_space()
            .clone();
        
        // Get subscription cache for notification support
        let subscriptions = self.handle.subscriptions().clone();

        // Register tags in address space
        {
            let mut space = address_space.write();
            let objects_folder: NodeId = ObjectId::ObjectsFolder.into();

            for entry in self.registry.entries() {
                register_tag_entry(&mut space, &objects_folder, entry);
            }
        }

        // Create PLC client and spawn poller
        let client = snap7_client::S7Client::connect(
            self.plc_addr,
            Default::default()
        ).await
            .map_err(|e| Error::Plc(e))?;

        let registry = self.registry.clone();
        let shutdown_rx = self.shutdown_tx.subscribe();
        let write_rx = self.write_rx;

        let _poller_handle = PlcPoller::spawn(
            client,
            registry,
            address_space.clone(),
            subscriptions,
            self.poll_interval,
            shutdown_rx,
            write_rx,
        );

        // Run OPC-UA server
        self.server.run().await
            .map_err(|e| Error::OpcUa(e))?;

        Ok(())
    }
}

/// Register a single tag entry as an OPC-UA variable node
///
/// Variables are created with proper access levels to support:
/// - Read operations (for polling data)
/// - Write operations (if the tag is configured as writable)
/// - Subscriptions (OPC-UA clients can subscribe to value changes)
fn register_tag_entry(
    space: &mut opcua::server::address_space::AddressSpace,
    objects_folder: &NodeId,
    entry: &TagEntry,
) {
    use snap7_client::proto::s7::header::TransportSize;

    // Determine the data type based on transport size
    let data_type = match entry.address.transport {
        TransportSize::Bit => DataTypeId::Boolean,
        TransportSize::Byte | TransportSize::Word | TransportSize::DWord => DataTypeId::UInt32,
        TransportSize::Int | TransportSize::DInt => DataTypeId::Int32,
        TransportSize::Real => DataTypeId::Float,
        TransportSize::Char | TransportSize::Date | TransportSize::Tod | TransportSize::Time | TransportSize::S5Time | TransportSize::DtL => DataTypeId::String,
    };

    let node_id = entry.node_id.clone();
    let tag_name = entry.name.clone();

    // Set access level based on whether the tag is writable
    let access_level = if entry.writable {
        // Read + Write + Subscription
        AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE
    } else {
        // Read-only + Subscription
        AccessLevel::CURRENT_READ
    };

    // Insert variable under ObjectsFolder with proper access levels
    // Access level controls what clients can do with this variable
    VariableBuilder::new(&node_id, &tag_name, LocalizedText::new("", &tag_name))
        .data_type(data_type)
        .value(Variant::Empty)
        .access_level(access_level)
        .user_access_level(access_level)
        .organized_by(objects_folder)
        .insert(space);
}
