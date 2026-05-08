use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{InfoAction, InfoArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: InfoArgs) -> Result<()> {
    match args.action {
        InfoAction::OrderCode => {
            let oc = client.get_order_code().await?;
            println!("Order code: {}", oc.code);
            println!("Version:    {}.{}.{}", oc.v1, oc.v2, oc.v3);
        }
        InfoAction::CpuInfo => {
            let ci = client.get_cpu_info().await?;
            println!("Module type:   {}", ci.module_type);
            println!("Serial number: {}", ci.serial_number);
            println!("AS name:       {}", ci.as_name);
            println!("Copyright:     {}", ci.copyright);
            println!("Module name:   {}", ci.module_name);
            println!("Protocol:      {} {}", ci.protocol,
                match ci.protocol {
                    snap7_client::Protocol::S7 => "(S7-300/400/1200)",
                    snap7_client::Protocol::S7Plus => "(S7-1200/1500)",
                });
        }
        InfoAction::CpInfo => {
            let cp = client.get_cp_info().await?;
            println!("Max PDU length:    {}", cp.max_pdu_len);
            println!("Max connections:   {}", cp.max_connections);
            println!("Max MPI rate:      {}", cp.max_mpi_rate);
            println!("Max bus rate:      {}", cp.max_bus_rate);
        }
        InfoAction::ModuleList => {
            let modules = client.read_module_list().await?;
            for (i, m) in modules.iter().enumerate() {
                println!("[{}] module_type=0x{:04X}", i, m.module_type);
            }
            if modules.is_empty() {
                println!("(empty module list)");
            }
        }
    }
    Ok(())
}
