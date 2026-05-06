use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

pub async fn run(client: &S7Client<TcpTransport>) -> Result<()> {
    let info = client
        .read_szl(0x0011, 0x0000)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Connected. SZL 0x0011 returned {} bytes.", info.data.len());
    Ok(())
}
