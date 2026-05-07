use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use opcua::server::address_space::{AccessLevel, VariableBuilder};
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::simple_node_manager;
use opcua::server::{Server, ServerBuilder, ServerHandle, ANONYMOUS_USER_TOKEN_ID};
use opcua::types::{
    DataTypeId, LocalizedText, MessageSecurityMode, NodeId, ObjectId, Variant,
};
use snap7_client::types::ConnectParams;
use tokio::sync::mpsc;

use crate::config::{GatewayConfig, OpcSecurityConfig};
use crate::error::{Error, Result};
use crate::poller::{PlcPoller, WriteCommand};
use crate::registry::TagRegistry;
use crate::registry::TagEntry;

/// OPC-UA Gateway — bridges one or more S7 PLCs to OPC-UA clients.
pub struct Gateway {
    server: Server,
    handle: ServerHandle,
    registries: Vec<Arc<TagRegistry>>,
    plc_addrs: Vec<(String, u64)>, // (addr, poll_interval_ms) per PLC
    endpoint_url: String,
    #[allow(dead_code)]
    write_tx: mpsc::Sender<WriteCommand>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl Gateway {
    pub async fn new(config: GatewayConfig) -> Result<Self> {
        let plcs = config.plc_configs();
        if plcs.is_empty() {
            return Err(Error::Config(
                "at least one PLC must be configured (use plc_addr + [[tags]] or [[plcs]])".into(),
            ));
        }

        // Parse security config
        let (sec_policy, sec_mode) = parse_security(&config.opc_security)?;

        // Build per-PLC registries + address map
        let mut registries: Vec<Arc<TagRegistry>> = Vec::new();
        let mut plc_addrs: Vec<(String, u64)> = Vec::new();
        for (i, plc) in plcs.iter().enumerate() {
            let reg = Arc::new(
                TagRegistry::from_specs(&plc.tags, i)
                    .map_err(|e| Error::Config(e.to_string()))?,
            );
            registries.push(reg);
            plc_addrs.push((plc.addr.clone(), plc.poll_interval_ms));
        }

        // Merge all tag entries into a unified registry for address-space registration
        let _unified_tags: Vec<TagEntry> = registries
            .iter()
            .flat_map(|r| r.entries().cloned())
            .collect();

        // Namespace metadata
        let namespace_meta = NamespaceMetadata {
            namespace_uri: "urn:snap7-opcua-gateway:tags".to_string(),
            namespace_index: 2,
            ..Default::default()
        };

        let opc_url = url::Url::parse(&config.opc_endpoint)
            .map_err(|e| Error::Config(format!("invalid opc_endpoint URL: {}", e)))?;
        let endpoint_path = opc_url.path();

        // Build OPC-UA server
        let mut server_builder = ServerBuilder::new()
            .application_name("snap7-opcua-gateway")
            .application_uri("urn:snap7-opcua-gateway")
            .host("0.0.0.0")
            .port(4840)
            .discovery_urls(vec![config.opc_endpoint.clone()])
            .with_node_manager(simple_node_manager(namespace_meta, "snap7-gateway"));

        // Configure endpoint with security
        server_builder = server_builder.add_endpoint(
            "none",
            (
                endpoint_path,
                sec_policy,
                sec_mode,
                &[ANONYMOUS_USER_TOKEN_ID] as &[&str],
            ),
        );

        let (server, handle) = server_builder
            .build()
            .map_err(|e| Error::OpcUa(e.to_string()))?;

        let endpoint_url = config.opc_endpoint.clone();
        let (write_tx, _write_rx) = mpsc::channel::<WriteCommand>(100);
        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Ok(Self {
            server,
            handle,
            registries,
            plc_addrs,
            endpoint_url,
            write_tx,
            shutdown_tx,
        })
    }

    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    pub async fn run(self) -> Result<()> {
        let address_space = self
            .handle
            .node_managers()
            .get_of_type::<opcua::server::node_manager::memory::SimpleNodeManager>()
            .ok_or_else(|| Error::OpcUa("Failed to get node manager".to_string()))?
            .address_space()
            .clone();

        let subscriptions = self.handle.subscriptions().clone();

        // Register all tags in address space
        {
            let mut space = address_space.write();
            let objects_folder: NodeId = ObjectId::ObjectsFolder.into();
            for reg in &self.registries {
                for entry in reg.entries() {
                    register_tag_entry(&mut space, &objects_folder, entry);
                }
            }
        }

        // Spawn one poller per PLC
        let shutdown_rx = self.shutdown_tx.subscribe();
        for (plc_idx, registry) in self.registries.into_iter().enumerate() {
            let (addr, poll_ms) = self.plc_addrs[plc_idx].clone();
            let registry = registry;
            let space = address_space.clone();
            let subs = subscriptions.clone();
            let shutdown = shutdown_rx.clone();
            let (_write_tx, write_rx) = mpsc::channel::<WriteCommand>(100);

            let plc_addr: SocketAddr = addr
                .parse()
                .map_err(|_| Error::Config(format!("invalid PLC addr: {}", addr)))?;
            let client = snap7_client::S7Client::connect(plc_addr, ConnectParams::default())
                .await
                .map_err(|e| Error::Plc(e))?;

            tokio::spawn(async move {
                PlcPoller::spawn(
                    client,
                    registry,
                    space,
                    subs,
                    Duration::from_millis(poll_ms),
                    shutdown,
                    write_rx,
                )
                .await
                .ok();
            });
        }

        // Run OPC-UA server (blocks until shutdown)
        self.server.run().await.map_err(|e| Error::OpcUa(e))?;
        Ok(())
    }
}

fn register_tag_entry(
    space: &mut opcua::server::address_space::AddressSpace,
    objects_folder: &NodeId,
    entry: &TagEntry,
) {
    use snap7_client::proto::s7::header::TransportSize;

    let data_type = match entry.address.transport {
        TransportSize::Bit => DataTypeId::Boolean,
        TransportSize::Byte | TransportSize::Word | TransportSize::DWord => DataTypeId::UInt32,
        TransportSize::Int | TransportSize::DInt => DataTypeId::Int32,
        TransportSize::Real => DataTypeId::Float,
        _ => DataTypeId::String,
    };

    let node_id = entry.node_id.clone();
    let tag_name = entry.name.clone();

    let access_level = if entry.writable {
        AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE
    } else {
        AccessLevel::CURRENT_READ
    };

    VariableBuilder::new(&node_id, &tag_name, LocalizedText::new("", &tag_name))
        .data_type(data_type)
        .value(Variant::Empty)
        .access_level(access_level)
        .user_access_level(access_level)
        .organized_by(objects_folder)
        .insert(space);
}

fn parse_security(
    sec: &OpcSecurityConfig,
) -> Result<(
    opcua::crypto::SecurityPolicy,
    MessageSecurityMode,
)> {
    let policy = match sec.policy.to_lowercase().as_str() {
        "none" => opcua::crypto::SecurityPolicy::None,
        "basic128" => opcua::crypto::SecurityPolicy::Basic128Rsa15,
        "basic256" => opcua::crypto::SecurityPolicy::Basic256,
        "basic256sha256" => opcua::crypto::SecurityPolicy::Basic256Sha256,
        _ => {
            return Err(Error::Config(format!(
                "unsupported security policy: {}. Use None, Basic128, Basic256, or Basic256Sha256",
                sec.policy
            )));
        }
    };

    let mode = match sec.mode.to_lowercase().as_str() {
        "none" => MessageSecurityMode::None,
        "sign" => MessageSecurityMode::Sign,
        "signandencrypt" => MessageSecurityMode::SignAndEncrypt,
        _ => {
            return Err(Error::Config(format!(
                "unsupported security mode: {}. Use None, Sign, or SignAndEncrypt",
                sec.mode
            )));
        }
    };

    Ok((policy, mode))
}
