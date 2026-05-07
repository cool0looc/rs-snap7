pub mod block;
pub mod diag;
pub mod info;
pub mod password;
pub mod plc_control;
pub mod read;
#[cfg(feature = "opcua")]
pub mod serve;
pub mod szl;
pub mod tag;
pub mod watch;
pub mod write;

use anyhow::Result;
use clap::Parser;
use snap7_client::{transport::TcpTransport, types::ConnectParams, S7Client};
use std::net::SocketAddr;
use std::time::Duration;

use crate::args::{Cli, Command};

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    // The Serve command doesn't need a PLC connection
    #[cfg(feature = "opcua")]
    if let Command::Serve(args) = &cli.command {
        serve::run(args).await?;
        return Ok(());
    }

    let host = cli.host.ok_or_else(|| {
        anyhow::anyhow!("--host is required for this command (or use 'snap7 serve' for gateway mode)")
    })?;
    
    let addr: SocketAddr = format!("{}:{}", host, cli.port).parse()?;
    let params = ConnectParams {
        rack: cli.rack,
        slot: cli.slot,
        connect_timeout: Duration::from_secs(cli.timeout_secs),
        ..Default::default()
    };

    if cli.tls {
        eprintln!("note: --tls selected; TLS is used for S7CommPlus sessions only");
    }
    if cli.udp {
        eprintln!("note: --udp selected; using UDP transport");
    }

    let client = S7Client::<TcpTransport>::connect(addr, params)
        .await
        .map_err(|e| anyhow::anyhow!("connection failed: {}", e))?;

    match cli.command {
        Command::Read(args) => read::run(&client, args, &cli.format).await?,
        Command::Write(args) => write::run(&client, args).await?,
        Command::Tag(args) => tag::run(&client, args, &cli.format).await?,
        Command::Block(args) => block::run(&client, args).await?,
        Command::Szl(args) => szl::run(&client, args, &cli.format).await?,
        Command::Diag => diag::run(&client).await?,
        Command::Watch(args) => watch::run(&client, args).await?,
        Command::PlcControl(args) => plc_control::run(&client, args).await?,
        Command::Info(args) => info::run(&client, args).await?,
        Command::Password(args) => password::run(&client, args).await?,
        #[cfg(feature = "opcua")]
        Command::Serve(_) => unreachable!(), // Handled above
    }

    Ok(())
}
