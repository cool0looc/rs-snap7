//! OPC-UA Subscription Demo
//!
//! Connects to the gateway_demo OPC-UA server and subscribes to Temperature,
//! Humidity, and Pressure variables.  Prints every value change notification.
//!
//! Usage:
//!   cargo run --features opcua --bin opcua_subscriber [-- [ENDPOINT]]
//!
//! Defaults to opc.tcp://127.0.0.1:4840/gateway.

use opcua::client::{ClientBuilder, DataChangeCallback, IdentityToken, MonitoredItem};
use opcua::types::{
    DataValue, EndpointDescription, MessageSecurityMode, MonitoredItemCreateRequest, NodeId,
    TimestampsToReturn, UserTokenPolicy,
};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let endpoint_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "opc.tcp://127.0.0.1:4840/gateway".to_string());

    println!("=== OPC-UA Subscription Demo ===");
    println!("Connecting to: {endpoint_url}");
    println!();

    let mut client = ClientBuilder::new()
        .application_name("snap7-opcua-subscriber")
        .application_uri("urn:snap7-opcua-subscriber")
        .trust_server_certs(true)
        .session_retry_limit(3)
        .client()
        .expect("failed to build OPC-UA client");

    let endpoint: EndpointDescription = (
        endpoint_url.as_str(),
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    )
        .into();

    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, IdentityToken::Anonymous)
        .await
        .expect("failed to connect to OPC-UA server");

    let handle = event_loop.spawn();

    println!("Waiting for session...");
    session.wait_for_connection().await;
    println!("Connected!\n");

    // Subscribe at 1 s interval — gateway polls PLC every 500 ms
    let sub_id = session
        .create_subscription(
            Duration::from_secs(1),
            10,   // max_notifications_per_publish
            30,   // lifetime_count
            0,    // max_keep_alive_count (server chooses)
            0,    // priority
            true, // publishing_enabled
            DataChangeCallback::new(|value: DataValue, item: &MonitoredItem| {
                let node_name = item.item_to_monitor().node_id.to_string();
                // Extract the string identifier for a cleaner name
                let name = node_name
                    .split("s=")
                    .nth(1)
                    .unwrap_or(&node_name)
                    .trim_end_matches('"');
                match value.value {
                    Some(v) => println!("[notification] {name:<15} = {v:?}"),
                    None => {
                        let status = value
                            .status
                            .map(|s| format!("{s:?}"))
                            .unwrap_or_else(|| "unknown".into());
                        println!("[notification] {name:<15} = <no value, status={status}>");
                    }
                }
            }),
        )
        .await
        .expect("failed to create subscription");

    println!("Subscription {sub_id} created. Monitoring:");

    // Monitor Temperature, Humidity, Pressure (all in namespace 2 as string IDs)
    let node_names = ["Temperature", "Humidity", "Pressure"];
    let items: Vec<MonitoredItemCreateRequest> = node_names
        .iter()
        .map(|name| {
            println!("  ns=2;s={name}");
            NodeId::new(2, *name).into()
        })
        .collect();

    println!();

    let results = session
        .create_monitored_items(sub_id, TimestampsToReturn::Both, items)
        .await
        .expect("failed to create monitored items");

    for (name, result) in node_names.iter().zip(results.iter()) {
        if result.result.status_code.is_good() {
            println!("  ✓ {name} monitored (id={})", result.result.monitored_item_id);
        } else {
            eprintln!("  ✗ {name} failed: {:?}", result.result.status_code);
        }
    }

    println!("\nListening for changes (Ctrl-C to stop)...\n");

    tokio::select! {
        _ = handle => println!("Session closed by server."),
        _ = tokio::signal::ctrl_c() => println!("\nStopping."),
    }
}
