//! PLC Batch Reader - Read PLC tags via OPC-UA gateway with push subscriptions
//!
//! Usage:
//!   plc_batch_reader -H <PLC_IP> [CONFIG_FILE] [--rate RATE] [--format FORMAT]
//!
//! Example:
//!   plc_batch_reader -H 10.139.25.15 /tmp/topic-3-config.json

use clap::Parser;
use opcua::client::{ClientBuilder, DataChangeCallback, IdentityToken, MonitoredItem};
use opcua::types::{
    DataValue, EndpointDescription, MessageSecurityMode, MonitoredItemCreateRequest, NodeId,
    ReadValueId, TimestampsToReturn, UserTokenPolicy, AttributeId,
};
use serde::Deserialize;
use snap7_opcua_gateway::{GatewayConfig, Gateway, TagSpec};
use snap7_client::proto::s7::header::TransportSize;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── JSON config types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
struct TagConfig {
    idx: serde_json::Value,
    address: String,
    tagname: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    unit: String,
    #[serde(default = "default_update_rate")]
    update_rate: u64,
}

fn default_update_rate() -> u64 {
    2500
}

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "plc_batch_reader", version = "2.0")]
struct Args {
    /// PLC IP address
    #[arg(short = 'H', long = "host")]
    host: String,

    /// PLC port (default: 102)
    #[arg(short = 'p', long = "port", default_value = "102")]
    port: u16,

    /// Rack number (default: 0)
    #[arg(short = 'r', long = "rack", default_value = "0")]
    rack: u8,

    /// Slot number (default: 2)
    #[arg(short = 's', long = "slot", default_value = "2")]
    slot: u8,

    /// Config file path (JSON)
    #[arg(default_value = "docs/1bd6e19a-ced4-474d-aa79-fc58e4e50a6d/topic-3-config.json")]
    config_file: String,

    /// Subscription publishing interval in ms (default: 1000)
    #[arg(short = 'R', long = "rate", default_value = "1000")]
    rate: u64,

    /// Output format: text, csv, json
    #[arg(short = 'f', long = "format", default_value = "text")]
    format: String,

    /// Read current values once and exit (no subscription)
    #[arg(short = '1', long = "once")]
    once: bool,

    /// Gateway poll interval in ms (default: 500)
    #[arg(long = "poll", default_value = "500")]
    poll_ms: u64,

    /// OPC-UA server port (default: auto-select 14840)
    #[arg(long = "opc-port", default_value = "14840")]
    opc_port: u16,
}

// ── Address parsing ───────────────────────────────────────────────────────────

/// Returns (db, byte_offset, TransportSize, byte_length, tag_format_for_gateway)
/// The tag_format is the `DB{n},{TYPE}{offset}` string accepted by TagRegistry.
fn parse_address(addr: &str) -> Option<(u16, u32, TransportSize, u16, String)> {
    // Formats: DB70.DINT0  DB70.REAL16  DB70.INT4  DB70.WORD8  DB70.DWORD12
    //          DB70.BYTE0  DB70.DBX332.0
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() < 2 || !parts[0].starts_with("DB") {
        return None;
    }
    let db: u16 = parts[0][2..].parse().ok()?;
    let type_part = parts[1];

    let (ts, byte_len, type_prefix, offset_digits): (TransportSize, u16, &str, &str) =
        if let Some(rest) = type_part.strip_prefix("DINT") {
            (TransportSize::DInt, 4, "DINT", rest)
        } else if let Some(rest) = type_part.strip_prefix("REAL") {
            (TransportSize::Real, 4, "REAL", rest)
        } else if let Some(rest) = type_part.strip_prefix("DWORD") {
            (TransportSize::DWord, 4, "DWORD", rest)
        } else if let Some(rest) = type_part.strip_prefix("WORD") {
            (TransportSize::Word, 2, "WORD", rest)
        } else if let Some(rest) = type_part.strip_prefix("INT") {
            (TransportSize::Int, 2, "INT", rest)
        } else if let Some(rest) = type_part.strip_prefix("BYTE") {
            (TransportSize::Byte, 1, "BYTE", rest)
        } else if let Some(rest) = type_part.strip_prefix("DBX") {
            // DBX332.0 — use BYTE read for bit tags
            let byte_off: u32 = rest.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let tag = format!("DB{db},BYTE{byte_off}");
            return Some((db, byte_off, TransportSize::Bit, 1, tag));
        } else {
            return None;
        };

    let byte_off: u32 = offset_digits.parse().unwrap_or(0);
    let tag = format!("DB{db},{type_prefix}{byte_off}");
    Some((db, byte_off, ts, byte_len, tag))
}

// ── Value formatting ──────────────────────────────────────────────────────────

fn format_variant(v: &opcua::types::Variant) -> String {
    match v {
        opcua::types::Variant::Float(f)  => format!("{f:.2}"),
        opcua::types::Variant::Int32(i)  => format!("{i}"),
        opcua::types::Variant::Int16(i)  => format!("{i}"),
        opcua::types::Variant::UInt32(u) => format!("0x{u:08X}"),
        opcua::types::Variant::UInt16(u) => format!("{u}"),
        opcua::types::Variant::Byte(b)   => format!("{b}"),
        opcua::types::Variant::Boolean(b) => format!("{b}"),
        other => format!("{other:?}"),
    }
}

fn print_row(format: &str, tag: &TagConfig, value: &str) {
    match format {
        "text" => println!(
            "{:<8} {:<22} {:<15} {:<15} {:<10}",
            tag.idx.to_string().trim_matches('"'),
            tag.tagname,
            tag.address,
            value,
            tag.unit
        ),
        "csv" => println!(
            "{},{},{},{},{},{}",
            tag.idx.to_string().trim_matches('"'),
            tag.tagname,
            tag.address,
            value,
            tag.unit,
            tag.description.replace(',', ";")
        ),
        _ => {}
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    let config_content = fs::read_to_string(&args.config_file)?;
    let config: serde_json::Value = serde_json::from_str(&config_content)?;
    let tags: Vec<TagConfig> =
        serde_json::from_value(config.get("payload").cloned().unwrap_or_default())?;

    println!("PLC Batch Reader v2.0 (OPC-UA gateway mode)");
    println!("============================================");
    println!("PLC:        {}:{}", args.host, args.port);
    println!("Rack/Slot:  {}/{}", args.rack, args.slot);
    println!("Config:     {} ({} tags)", args.config_file, tags.len());
    println!("Format:     {}", args.format);
    println!("Mode:       {}", if args.once { "read once" } else { "subscription" });
    println!();

    // ── Build TagSpec list from parsed addresses ───────────────────────────────
    // tag name = tagname (unique per tag, used as OPC-UA node string id)
    // We deduplicate names in case of duplicates to avoid NodeId collisions.
    let mut seen_names: HashMap<String, usize> = HashMap::new();
    let mut tag_specs: Vec<(usize, TagSpec, &TagConfig)> = Vec::new();

    for (i, tag) in tags.iter().enumerate() {
        let Some((_, _, _, _, gateway_tag)) = parse_address(&tag.address) else {
            continue;
        };
        // Ensure unique OPC-UA node name
        let count = seen_names.entry(tag.tagname.clone()).or_insert(0);
        let node_name = if *count == 0 {
            tag.tagname.clone()
        } else {
            format!("{}_{}", tag.tagname, count)
        };
        *count += 1;

        tag_specs.push((
            i,
            TagSpec { tag: gateway_tag, name: node_name, writable: false },
            tag,
        ));
    }

    println!("Valid tags: {}/{}", tag_specs.len(), tags.len());
    if tag_specs.is_empty() {
        eprintln!("No parseable tags found. Check your config file.");
        return Ok(());
    }

    // ── Start OPC-UA gateway ──────────────────────────────────────────────────
    let opc_endpoint = format!("opc.tcp://127.0.0.1:{}/plc", args.opc_port);
    let gateway_config = GatewayConfig {
        plc_addr: format!("{}:{}", args.host, args.port),
        opc_endpoint: opc_endpoint.clone(),
        poll_interval_ms: args.poll_ms,
        tags: tag_specs.iter().map(|(_, spec, _)| spec.clone()).collect(),
    };

    println!("Starting OPC-UA gateway on {} ...", opc_endpoint);
    let gateway = Gateway::new(gateway_config).await?;
    println!("Gateway ready. Connecting to PLC {}:{} ...\n", args.host, args.port);

    tokio::spawn(async move {
        if let Err(e) = gateway.run().await {
            eprintln!("[gateway] exited: {e:?}");
        }
    });

    // Give the gateway a moment to bind and connect before we try to attach
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // ── Connect OPC-UA client ─────────────────────────────────────────────────
    let mut client = ClientBuilder::new()
        .application_name("plc-batch-reader")
        .application_uri("urn:plc-batch-reader")
        .trust_server_certs(true)
        .session_retry_limit(3)
        .client()
        .map_err(|e| e.join(", "))?;

    let endpoint: EndpointDescription = (
        opc_endpoint.as_str(),
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    )
        .into();

    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, IdentityToken::Anonymous)
        .await?;

    let _loop_handle = event_loop.spawn();
    session.wait_for_connection().await;
    println!("OPC-UA session established.\n");

    // ── Print header ──────────────────────────────────────────────────────────
    match args.format.as_str() {
        "text" => {
            println!(
                "{:<8} {:<22} {:<15} {:<15} {:<10}",
                "idx", "tagname", "address", "value", "unit"
            );
            println!("{}", "-".repeat(75));
        }
        "csv" => println!("idx,tagname,address,value,unit,description"),
        _ => {}
    }

    // ── Once mode: Read service (no subscription) ─────────────────────────────
    if args.once {
        let nodes: Vec<ReadValueId> = tag_specs
            .iter()
            .map(|(_, spec, _)| ReadValueId {
                node_id: NodeId::new(2, spec.name.clone()),
                attribute_id: AttributeId::Value as u32,
                ..Default::default()
            })
            .collect();

        let results = session.read(&nodes, TimestampsToReturn::Neither, 0.0).await?;

        for (result, (_, spec, tag)) in results.iter().zip(tag_specs.iter()) {
            let value = result
                .value
                .as_ref()
                .map(format_variant)
                .unwrap_or_else(|| "N/A".into());
            print_row(&args.format, tag, &value);
            let _ = spec; // suppress unused warning
        }

        return Ok(());
    }

    // ── Subscription mode ─────────────────────────────────────────────────────
    // Build a name → (idx_in_tags, &TagConfig) lookup for the callback.
    // The callback receives the node name as the string NodeId.
    let name_map: Arc<HashMap<String, (usize, TagConfig)>> = Arc::new(
        tag_specs
            .iter()
            .map(|(i, spec, tag)| (spec.name.clone(), (*i, (*tag).clone())))
            .collect(),
    );

    let format_str = Arc::new(args.format.clone());

    // Track last-printed value per node to suppress duplicates in quiet intervals
    let last_values: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));

    let name_map_cb = Arc::clone(&name_map);
    let format_cb   = Arc::clone(&format_str);
    let last_cb     = Arc::clone(&last_values);

    let sub_id = session
        .create_subscription(
            Duration::from_millis(args.rate),
            100,  // max_notifications_per_publish
            30,   // lifetime_count
            0,    // max_keep_alive_count
            0,    // priority
            true,
            DataChangeCallback::new(move |dv: DataValue, item: &MonitoredItem| {
                let node_str = item.item_to_monitor().node_id.to_string();
                // NodeId string is e.g. `ns=2;s=MotorSpeed`
                let name = node_str
                    .split("s=")
                    .nth(1)
                    .unwrap_or(&node_str)
                    .trim_matches('"')
                    .to_string();

                let Some((_, tag)) = name_map_cb.get(&name) else { return };

                let value = dv
                    .value
                    .as_ref()
                    .map(format_variant)
                    .unwrap_or_else(|| "N/A".into());

                // Suppress if value unchanged since last print
                {
                    let mut lv = last_cb.lock().unwrap();
                    if lv.get(&name) == Some(&value) {
                        return;
                    }
                    lv.insert(name, value.clone());
                }

                print_row(&format_cb, tag, &value);
            }),
        )
        .await?;

    let monitored: Vec<MonitoredItemCreateRequest> = tag_specs
        .iter()
        .map(|(_, spec, _)| NodeId::new(2, spec.name.clone()).into())
        .collect();

    let results = session
        .create_monitored_items(sub_id, TimestampsToReturn::Both, monitored)
        .await?;

    let mut ok = 0usize;
    let mut fail = 0usize;
    for ((_i, spec, _tag), result) in tag_specs.iter().zip(results.iter()) {
        if result.result.status_code.is_good() {
            ok += 1;
        } else {
            eprintln!("  ✗ {} failed: {:?}", spec.name, result.result.status_code);
            fail += 1;
        }
    }
    eprintln!("Monitoring {ok} tags ({fail} failed). Waiting for changes (Ctrl-C to stop)...\n");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => println!("\nStopped."),
    }

    Ok(())
}
