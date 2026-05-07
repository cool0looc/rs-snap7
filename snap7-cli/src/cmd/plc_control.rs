use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{PlcAction, PlcControlArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: PlcControlArgs) -> Result<()> {
    match args.action {
        PlcAction::Stop => {
            client.plc_stop().await?;
            println!("ok  – plc stop command sent");
        }
        PlcAction::HotStart => {
            client.plc_hot_start().await?;
            println!("ok  – plc hot-start command sent");
        }
        PlcAction::ColdStart => {
            client.plc_cold_start().await?;
            println!("ok  – plc cold-start command sent");
        }
        PlcAction::Status => {
            let status = client.get_plc_status().await?;
            let label = match status {
                snap7_client::PlcStatus::Run => "RUN",
                snap7_client::PlcStatus::Stop => "STOP",
                snap7_client::PlcStatus::Unknown => "UNKNOWN",
            };
            println!("PLC status: {}", label);
        }
    }
    Ok(())
}
