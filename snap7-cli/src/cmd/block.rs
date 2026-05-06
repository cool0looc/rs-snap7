use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::BlockArgs;

pub async fn run(_client: &S7Client<TcpTransport>, _args: BlockArgs) -> Result<()> {
    println!("block operations");
    Ok(())
}
