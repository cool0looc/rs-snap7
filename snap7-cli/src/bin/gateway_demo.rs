//! OPC-UA Gateway Demo
//!
//! Starts a local sensor_server simulator and the snap7-opcua-gateway, then
//! waits for Ctrl-C.  Run the companion `opcua_subscriber` in another terminal
//! to see live subscription notifications.
//!
//! Usage:
//!   cargo run --features opcua --bin gateway_demo [-- [PLC_PORT [OPC_PORT]]]
//!
//! Defaults: PLC on 10200, OPC-UA on 4840.

use snap7_opcua_gateway::{GatewayConfig, Gateway, TagSpec};
use snap7_server::{DataStore, S7Server, ServerConfig};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    let plc_port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10200);

    let opc_port: u16 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4840);

    println!("=== snap7 OPC-UA Gateway Demo ===");
    println!("  PLC simulator : 0.0.0.0:{plc_port}  (S7 protocol)");
    println!("  OPC-UA server : opc.tcp://0.0.0.0:{opc_port}/gateway");
    println!();
    println!("  Subscribe to these nodes (ns=2):");
    println!("    ns=2;s=Temperature    REAL  DB1 @ byte 0");
    println!("    ns=2;s=Humidity       REAL  DB2 @ byte 0");
    println!("    ns=2;s=Pressure       REAL  DB3 @ byte 0");
    println!();
    println!("  Run the subscriber in another terminal:");
    println!("    cargo run --features opcua --bin opcua_subscriber");
    println!();

    // ── Sensor simulator ─────────────────────────────────────────────────────
    let store = DataStore::new();
    store.write_bytes(1, 0, &25.0_f32.to_be_bytes());
    store.write_bytes(2, 0, &60.0_f32.to_be_bytes());
    store.write_bytes(3, 0, &101.325_f32.to_be_bytes());

    let store_for_update = store.clone();
    tokio::task::spawn_blocking(move || {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let mut temperature = 25.0_f32;
        let mut humidity = 60.0_f32;
        let mut pressure = 101.325_f32;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            temperature = (temperature + rng.gen_range(-0.3..0.3)).clamp(20.0, 30.0);
            humidity = (humidity + rng.gen_range(-0.5..0.5)).clamp(40.0, 80.0);
            pressure = (pressure + rng.gen_range(-0.1..0.1)).clamp(100.0, 105.0);
            store_for_update.write_bytes(1, 0, &temperature.to_be_bytes());
            store_for_update.write_bytes(2, 0, &humidity.to_be_bytes());
            store_for_update.write_bytes(3, 0, &pressure.to_be_bytes());
        }
    });

    let plc_addr: SocketAddr = format!("0.0.0.0:{plc_port}").parse().unwrap();
    let plc_server = S7Server::bind(ServerConfig {
        bind_addr: plc_addr,
        max_connections: 4,
    })
    .await
    .expect("failed to bind PLC simulator");

    println!("PLC simulator listening on 0.0.0.0:{plc_port}");

    let (plc_stop_tx, plc_stop_rx) = watch::channel(false);
    let store_for_server = store.clone();
    tokio::spawn(async move {
        tokio::select! {
            res = plc_server.serve(store_for_server) => {
                if let Err(e) = res { eprintln!("[plc] server error: {e}"); }
            }
            _ = async { let mut rx = plc_stop_rx; let _ = rx.changed().await; } => {}
        }
    });

    // ── OPC-UA gateway ────────────────────────────────────────────────────────
    let config = GatewayConfig {
        plc_addr: format!("127.0.0.1:{plc_port}"),
        opc_endpoint: format!("opc.tcp://0.0.0.0:{opc_port}/gateway"),
        poll_interval_ms: 500,
        tags: vec![
            TagSpec { tag: "DB1,REAL0".into(), name: "Temperature".into(), writable: false },
            TagSpec { tag: "DB2,REAL0".into(), name: "Humidity".into(),    writable: false },
            TagSpec { tag: "DB3,REAL0".into(), name: "Pressure".into(),    writable: false },
        ],
    };

    println!("Starting OPC-UA gateway...");
    let gateway = Gateway::new(config).await.expect("failed to create gateway");
    println!("OPC-UA gateway ready: {}", gateway.endpoint_url());
    println!("\nPress Ctrl-C to stop.\n");

    tokio::select! {
        result = gateway.run() => {
            eprintln!("Gateway exited: {:?}", result);
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nStopping...");
            let _ = plc_stop_tx.send(true);
        }
    }
}
