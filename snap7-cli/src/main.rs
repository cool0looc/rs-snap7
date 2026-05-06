mod args;
mod cmd;
mod output;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cmd::run().await
}
