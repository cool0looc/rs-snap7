use anyhow::Result;
use snap7_client::{transport::TcpTransport, S7Client};

use crate::args::{PasswordAction, PasswordArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: PasswordArgs) -> Result<()> {
    match args.action {
        PasswordAction::Set { password } => {
            client.set_session_password(&password).await?;
            println!("ok  – session password set");
        }
        PasswordAction::Clear => {
            client.clear_session_password().await?;
            println!("ok  – session password cleared");
        }
    }
    Ok(())
}
