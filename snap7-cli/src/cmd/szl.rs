use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{OutputFormat, SzlArgs};

pub async fn run(
    client: &S7Client<TcpTransport>,
    args: SzlArgs,
    format: &OutputFormat,
) -> Result<()> {
    let info = client
        .read_szl(args.id, args.index)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    match format {
        OutputFormat::Json => {
            let hex: Vec<String> = info.data.iter().map(|b| format!("{:02X}", b)).collect();
            println!(
                "{{\"szl_id\":{},\"szl_index\":{},\"data\":[{}]}}",
                args.id,
                args.index,
                hex.join(",")
            );
        }
        _ => crate::output::print_bytes(&info.data, format),
    }
    Ok(())
}
