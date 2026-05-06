use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{OutputFormat, ReadArgs};
use crate::output::print_bytes;

pub async fn run(
    client: &S7Client<TcpTransport>,
    args: ReadArgs,
    format: &OutputFormat,
) -> Result<()> {
    let data = client
        .db_read(args.db, args.offset, args.size)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    print_bytes(&data, format);
    Ok(())
}
