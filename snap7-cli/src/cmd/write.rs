use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::WriteArgs;

pub async fn run(client: &S7Client<TcpTransport>, args: WriteArgs) -> Result<()> {
    let data =
        hex::decode(&args.data).map_err(|_| anyhow::anyhow!("invalid hex data: {}", args.data))?;
    client
        .db_write(args.db, args.offset, &data)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("ok");
    Ok(())
}
