//! OPC-UA Gateway serve subcommand

use anyhow::{Context, Result};

use crate::args::ServeArgs;

/// Run the OPC-UA gateway server
pub async fn run(args: &ServeArgs) -> Result<()> {
    use snap7_opcua_gateway::{Gateway, GatewayConfig};

    // Read the config file
    let toml_content = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading config file {}", args.config.display()))?;

    // Parse the config
    let config: GatewayConfig =
        toml::from_str(&toml_content).context("parsing gateway config TOML")?;

    println!(
        "Starting OPC-UA gateway:\n  PLC: {}\n  OPC endpoint: {}\n  Poll interval: {}ms\n  Tags: {}",
        config.plc_addr,
        config.opc_endpoint,
        config.poll_interval_ms,
        config.tags.len()
    );

    // Create and run the gateway
    let gateway = Gateway::new(config).await?;
    let endpoint = gateway.endpoint_url();
    println!("OPC-UA server listening on {}", endpoint);

    gateway.run().await?;

    Ok(())
}
