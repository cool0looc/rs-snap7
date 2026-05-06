use std::sync::Arc;
use std::time::Duration;

use opcua::server::address_space::AddressSpace;
use opcua::sync::RwLock;
use opcua::types::{AttributeId, DataValue, DateTime, NodeId, Variant};
use snap7_client::transport::TcpTransport;
use snap7_client::S7Client;
use tokio::sync::{mpsc, watch};
use tokio::time::interval;

use crate::error::{Error, Result};
use crate::registry::TagRegistry;

/// A write command from OPC-UA client to PLC
pub struct WriteCommand {
    /// The NodeId of the variable to write
    pub node_id: NodeId,
    /// The new value to write
    pub variant: Variant,
}

/// PlcPoller polls PLC tags and updates the OPC-UA address space.
///
/// This poller implements the data bridge between S7 PLC and OPC-UA clients:
/// 1. Periodically reads PLC data blocks via S7 protocol
/// 2. Updates OPC-UA variable values in the address space
/// 3. Forwards write commands from OPC-UA clients to PLC
///
/// ## Subscription Support
///
/// When variables are updated via `var.set_value()`, the OPC-UA server
/// automatically notifies subscribed clients. The polling interval determines
/// the maximum notification latency.
///
/// ## Write Operations
///
/// Write commands received from OPC-UA clients are processed in two ways:
/// - Prioritized: If a write arrives between polls, it's applied immediately
/// - Batched: Pending writes are drained at the start of each poll cycle
pub struct PlcPoller;

/// Type alias for the subscription cache
type Subscriptions = opcua::server::SubscriptionCache;

impl PlcPoller {
    /// Spawn a new poller task
    ///
    /// The poller will run until `shutdown` receives a value or an error occurs.
    /// Write commands received from `write_rx` are forwarded to the PLC.
    ///
    /// # Subscription Behavior
    /// 
    /// This poller supports OPC-UA subscriptions:
    /// - After updating values, it calls `subscriptions.notify_data_change()` 
    ///   to notify subscribed clients of the changes
    /// - Notifications are sent at the polling interval (determined by `poll_interval`)
    /// - For lower latency, reduce `poll_interval` or use event-driven updates
    pub fn spawn(
        client: S7Client<TcpTransport>,
        registry: Arc<TagRegistry>,
        address_space: Arc<RwLock<AddressSpace>>,
        subscriptions: Arc<Subscriptions>,
        poll_interval: Duration,
        mut shutdown: watch::Receiver<bool>,
        mut write_rx: mpsc::Receiver<WriteCommand>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(poll_interval);
            let mut client = client;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // First, drain all pending write commands
                        while let Ok(cmd) = write_rx.try_recv() {
                            if let Err(e) = Self::apply_write(&mut client, &registry, &address_space, cmd).await {
                                eprintln!("[poller] write error: {e}");
                            }
                        }
                        // Then poll all tags and notify subscriptions
                        match Self::poll_once(&mut client, &registry, &address_space).await {
                            Ok(updated) => {
                                // Notify subscribed clients of value changes
                                Self::notify_subscriptions(&subscriptions, updated);
                            }
                            Err(e) => {
                                eprintln!("[poller] PLC error: {e}; retrying in 2s");
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                        }
                    }
                    Some(cmd) = write_rx.recv() => {
                        if let Err(e) = Self::apply_write(&mut client, &registry, &address_space, cmd).await {
                            eprintln!("[poller] write error: {e}");
                        }
                    }
                    _ = shutdown.changed() => break,
                }
            }
        })
    }

    /// Notify subscription cache of value changes
    fn notify_subscriptions(
        subscriptions: &Arc<Subscriptions>,
        updated: Vec<(NodeId, Variant)>,
    ) {
        let now = DateTime::now();
        
        // Notify each updated variable
        for (node_id, variant) in updated {
            let data_value = DataValue::new_at(variant, now);
            subscriptions.notify_data_change(
                std::iter::once((data_value, &node_id, AttributeId::Value))
            );
        }
    }

    /// Apply a write command to the PLC
    async fn apply_write(
        client: &mut S7Client<TcpTransport>,
        registry: &TagRegistry,
        address_space: &Arc<RwLock<AddressSpace>>,
        cmd: WriteCommand,
    ) -> Result<()> {
        // Find the tag entry for this NodeId
        let entry = registry
            .find_by_node_id(&cmd.node_id)
            .ok_or_else(|| Error::OpcUa(format!("unknown node: {}", cmd.node_id)))?;

        if !entry.writable {
            return Err(Error::OpcUa(format!(
                "node {} is not writable",
                cmd.node_id
            )));
        }

        let (data, incoming_ts) = variant_to_bytes(&cmd.variant)
            .ok_or_else(|| Error::OpcUa("unsupported variant type for write".to_string()))?;

        // Reject if the incoming variant's transport size doesn't match the tag's declared type.
        // A mismatch means wrong byte width, which would corrupt adjacent PLC memory.
        if incoming_ts != entry.address.transport {
            return Err(Error::OpcUa(format!(
                "type mismatch for node {}: tag expects {:?}, got {:?}",
                cmd.node_id, entry.address.transport, incoming_ts,
            )));
        }

        // Write to PLC
        client
            .db_write(entry.address.db_number, entry.address.byte_offset, &data)
            .await?;

        // Update the address space with the new value - this triggers subscription notifications
        let mut space = address_space.write();
        if let Some(opcua::server::address_space::NodeType::Variable(var)) =
            space.find_mut(&entry.node_id)
        {
            let _ = var.set_value(&opcua::types::NumericRange::None, cmd.variant);
        }

        Ok(())
    }

    async fn poll_once(
        client: &mut S7Client<TcpTransport>,
        registry: &TagRegistry,
        address_space: &Arc<RwLock<AddressSpace>>,
    ) -> Result<Vec<(NodeId, Variant)>> {
        let mut updated = Vec::new();
        
        for entry in registry.entries() {
            let addr = &entry.address;
            let Some(size) = transport_size_bytes(addr.transport, addr.element_count) else {
                continue;
            };
            
            // Read data from PLC
            let data = client
                .db_read(addr.db_number, addr.byte_offset, size as u16)
                .await?;

            // Convert to OPC-UA variant
            if let Some(variant) = bytes_to_variant(&data, addr.transport) {
                // Update the address space - this triggers subscription notifications
                // for any subscribed OPC-UA clients
                let mut space = address_space.write();
                if let Some(opcua::server::address_space::NodeType::Variable(var)) =
                    space.find_mut(&entry.node_id)
                {
                    let _ = var.set_value(&opcua::types::NumericRange::None, variant.clone());
                    updated.push((entry.node_id.clone(), variant));
                }
            }
        }
        Ok(updated)
    }
}

/// Convert raw bytes to an OPC-UA Variant based on the transport size
pub(crate) fn bytes_to_variant(
    bytes: &[u8],
    ts: snap7_client::proto::s7::header::TransportSize,
) -> Option<Variant> {
    match ts {
        snap7_client::proto::s7::header::TransportSize::Real if bytes.len() >= 4 => {
            let v = f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            Some(Variant::Float(v))
        }
        snap7_client::proto::s7::header::TransportSize::DWord
        | snap7_client::proto::s7::header::TransportSize::DInt
            if bytes.len() >= 4 =>
        {
            let v = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            if ts == snap7_client::proto::s7::header::TransportSize::DInt {
                Some(Variant::Int32(v as i32))
            } else {
                Some(Variant::UInt32(v))
            }
        }
        snap7_client::proto::s7::header::TransportSize::Word if bytes.len() >= 2 => {
            let v = u16::from_be_bytes([bytes[0], bytes[1]]);
            Some(Variant::UInt16(v))
        }
        snap7_client::proto::s7::header::TransportSize::Int if bytes.len() >= 2 => {
            let v = i16::from_be_bytes([bytes[0], bytes[1]]);
            Some(Variant::Int16(v))
        }
        snap7_client::proto::s7::header::TransportSize::Byte if !bytes.is_empty() => {
            Some(Variant::Byte(bytes[0]))
        }
        snap7_client::proto::s7::header::TransportSize::Bit if !bytes.is_empty() => {
            Some(Variant::Boolean(bytes[0] != 0))
        }
        _ => None,
    }
}

/// Calculate the number of bytes needed for a given transport size.
/// Returns `None` for transport sizes that have no known byte width.
fn transport_size_bytes(
    ts: snap7_client::proto::s7::header::TransportSize,
    count: u16,
) -> Option<usize> {
    let unit = match ts {
        snap7_client::proto::s7::header::TransportSize::Real
        | snap7_client::proto::s7::header::TransportSize::DWord
        | snap7_client::proto::s7::header::TransportSize::DInt => 4,
        snap7_client::proto::s7::header::TransportSize::Word
        | snap7_client::proto::s7::header::TransportSize::Int => 2,
        snap7_client::proto::s7::header::TransportSize::Byte
        | snap7_client::proto::s7::header::TransportSize::Bit => 1,
        other => {
            eprintln!("[poller] unsupported transport size {other:?}, skipping tag");
            return None;
        }
    };
    Some(unit * count as usize)
}

/// Convert a Variant to big-endian PLC bytes, paired with the expected TransportSize.
pub(crate) fn variant_to_bytes(
    variant: &Variant,
) -> Option<(Vec<u8>, snap7_client::proto::s7::header::TransportSize)> {
    match variant {
        Variant::Float(f) => Some((
            f.to_be_bytes().to_vec(),
            snap7_client::proto::s7::header::TransportSize::Real,
        )),
        Variant::Int32(i) => Some((
            (*i as u32).to_be_bytes().to_vec(),
            snap7_client::proto::s7::header::TransportSize::DInt,
        )),
        Variant::UInt32(u) => Some((
            u.to_be_bytes().to_vec(),
            snap7_client::proto::s7::header::TransportSize::DWord,
        )),
        Variant::Int16(i) => Some((
            (*i as u16).to_be_bytes().to_vec(),
            snap7_client::proto::s7::header::TransportSize::Int,
        )),
        Variant::UInt16(u) => Some((
            u.to_be_bytes().to_vec(),
            snap7_client::proto::s7::header::TransportSize::Word,
        )),
        Variant::Byte(b) => Some((vec![*b], snap7_client::proto::s7::header::TransportSize::Byte)),
        Variant::Boolean(b) => Some((
            vec![if *b { 1 } else { 0 }],
            snap7_client::proto::s7::header::TransportSize::Bit,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snap7_client::proto::s7::header::TransportSize;

    #[test]
    fn decode_f32_big_endian() {
        // 1.0f32 in big-endian = 0x3F 0x80 0x00 0x00
        let bytes = [0x3F_u8, 0x80, 0x00, 0x00];
        let v = bytes_to_variant(&bytes, TransportSize::Real).unwrap();
        assert!(matches!(v, Variant::Float(f) if (f - 1.0f32).abs() < 1e-6));
    }

    #[test]
    fn decode_u16_big_endian() {
        let bytes = [0x01_u8, 0x00];
        let v = bytes_to_variant(&bytes, TransportSize::Word).unwrap();
        assert_eq!(v, Variant::UInt16(256));
    }

    #[test]
    fn decode_i16_negative() {
        let bytes = [0xFF_u8, 0x80]; // -128 as signed 16-bit
        let v = bytes_to_variant(&bytes, TransportSize::Int).unwrap();
        assert_eq!(v, Variant::Int16(-128));
    }

    #[test]
    fn decode_insufficient_bytes_returns_none() {
        let bytes = [0x01_u8]; // only 1 byte for a WORD
        assert!(bytes_to_variant(&bytes, TransportSize::Word).is_none());
    }

    #[test]
    fn decode_byte() {
        let bytes = [0x42_u8];
        let v = bytes_to_variant(&bytes, TransportSize::Byte).unwrap();
        assert_eq!(v, Variant::Byte(0x42));
    }

    #[test]
    fn decode_bit_true() {
        let bytes = [0x01_u8];
        let v = bytes_to_variant(&bytes, TransportSize::Bit).unwrap();
        assert_eq!(v, Variant::Boolean(true));
    }

    #[test]
    fn decode_bit_false() {
        let bytes = [0x00_u8];
        let v = bytes_to_variant(&bytes, TransportSize::Bit).unwrap();
        assert_eq!(v, Variant::Boolean(false));
    }

    #[test]
    fn decode_dint() {
        let bytes = [0x00_u8, 0x00, 0x00, 0x01]; // 1 as signed 32-bit
        let v = bytes_to_variant(&bytes, TransportSize::DInt).unwrap();
        assert_eq!(v, Variant::Int32(1));
    }

    #[test]
    fn decode_dword() {
        let bytes = [0x00_u8, 0x00, 0x00, 0x01]; // 1 as unsigned 32-bit
        let v = bytes_to_variant(&bytes, TransportSize::DWord).unwrap();
        assert_eq!(v, Variant::UInt32(1));
    }

    #[test]
    fn transport_size_bytes_real() {
        assert_eq!(transport_size_bytes(TransportSize::Real, 1), Some(4));
        assert_eq!(transport_size_bytes(TransportSize::Real, 2), Some(8));
    }

    #[test]
    fn transport_size_bytes_word() {
        assert_eq!(transport_size_bytes(TransportSize::Word, 1), Some(2));
        assert_eq!(transport_size_bytes(TransportSize::Word, 3), Some(6));
    }

    #[test]
    fn transport_size_bytes_byte() {
        assert_eq!(transport_size_bytes(TransportSize::Byte, 1), Some(1));
        assert_eq!(transport_size_bytes(TransportSize::Byte, 10), Some(10));
    }

    // Variant to bytes tests
    #[test]
    fn variant_to_bytes_float() {
        let variant = Variant::Float(1.0);
        let (bytes, ts) = variant_to_bytes(&variant).unwrap();
        assert_eq!(ts, TransportSize::Real);
        assert_eq!(bytes.as_slice(), &[0x3F, 0x80, 0x00, 0x00]);
    }

    #[test]
    fn variant_to_bytes_uint16() {
        let variant = Variant::UInt16(256);
        let (bytes, ts) = variant_to_bytes(&variant).unwrap();
        assert_eq!(ts, TransportSize::Word);
        assert_eq!(bytes.as_slice(), &[0x01, 0x00]);
    }

    #[test]
    fn variant_to_bytes_int16() {
        let variant = Variant::Int16(-128);
        let (bytes, ts) = variant_to_bytes(&variant).unwrap();
        assert_eq!(ts, TransportSize::Int);
        assert_eq!(bytes.as_slice(), &[0xFF, 0x80]);
    }

    #[test]
    fn variant_to_bytes_unsupported() {
        let variant = Variant::String("test".into());
        assert!(variant_to_bytes(&variant).is_none());
    }
}
