use anyhow::{bail, Result};
use snap7_client::{transport::TcpTransport, PlcDateTime, S7Client};

use crate::args::{ClockAction, ClockArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: ClockArgs) -> Result<()> {
    match args.action {
        ClockAction::Read => {
            let dt = client.read_clock().await?;
            println!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second, dt.millisecond
            );
        }
        ClockAction::Set { datetime, force } => {
            let dt = parse_datetime(&datetime)?;
            if !force {
                confirm_destructive("set PLC clock")?;
            }
            client.set_clock(&dt).await?;
            println!(
                "ok  – PLC clock set to {:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
            );
        }
        ClockAction::Sync { force } => {
            if !force {
                confirm_destructive("sync PLC clock to system time")?;
            }
            client.set_clock_to_now().await?;
            println!("ok  – PLC clock synced to system time");
        }
    }
    Ok(())
}

fn parse_datetime(s: &str) -> Result<PlcDateTime> {
    // Accept YYYY-MM-DDTHH:MM:SS or YYYY-MM-DD HH:MM:SS
    let s = s.replace(' ', "T");
    let parts: Vec<&str> = s.splitn(2, 'T').collect();
    if parts.len() != 2 {
        bail!("invalid datetime format — use YYYY-MM-DDTHH:MM:SS");
    }
    let date_parts: Vec<&str> = parts[0].split('-').collect();
    let time_parts: Vec<&str> = parts[1].split(':').collect();
    if date_parts.len() != 3 || time_parts.len() != 3 {
        bail!("invalid datetime format — use YYYY-MM-DDTHH:MM:SS");
    }
    let year: u16 = date_parts[0].parse().map_err(|_| anyhow::anyhow!("invalid year"))?;
    let month: u8 = date_parts[1].parse().map_err(|_| anyhow::anyhow!("invalid month"))?;
    let day: u8 = date_parts[2].parse().map_err(|_| anyhow::anyhow!("invalid day"))?;
    let hour: u8 = time_parts[0].parse().map_err(|_| anyhow::anyhow!("invalid hour"))?;
    let minute: u8 = time_parts[1].parse().map_err(|_| anyhow::anyhow!("invalid minute"))?;
    let second: u8 = time_parts[2].parse().map_err(|_| anyhow::anyhow!("invalid second"))?;

    if month < 1 || month > 12 {
        bail!("month must be 1–12");
    }
    if day < 1 || day > 31 {
        bail!("day must be 1–31");
    }
    if hour > 23 {
        bail!("hour must be 0–23");
    }
    if minute > 59 {
        bail!("minute must be 0–59");
    }
    if second > 59 {
        bail!("second must be 0–59");
    }

    Ok(PlcDateTime { year, month, day, hour, minute, second, millisecond: 0, weekday: 0 })
}

fn confirm_destructive(action: &str) -> Result<()> {
    eprint!("warning: about to {action}. Confirm? [y/N] ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    if input.trim().eq_ignore_ascii_case("y") {
        Ok(())
    } else {
        bail!("aborted");
    }
}
