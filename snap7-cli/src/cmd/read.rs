use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{AreaArg, OutputFormat, ReadArgs};
use crate::output::print_bytes;

pub async fn run(
    client: &S7Client<TcpTransport>,
    args: ReadArgs,
    format: &OutputFormat,
) -> Result<()> {
    let area: snap7_client::proto::s7::header::Area = args.area.clone().into();
    let data = match args.area.clone() {
        AreaArg::Db => {
            client
                .db_read(args.db, args.offset, args.size)
                .await
        }
        _ => {
            client
                .ab_read(area, args.db, args.offset, args.size)
                .await
        }
    }
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    print_bytes(&data, format);
    Ok(())
}
