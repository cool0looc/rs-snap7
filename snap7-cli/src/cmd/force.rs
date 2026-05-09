use anyhow::{bail, Result};
use snap7_client::{proto::s7::header::Area, transport::TcpTransport, S7Client};

use crate::args::{ForceAction, ForceArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: ForceArgs) -> Result<()> {
    match args.action {
        ForceAction::Set { address, value } => {
            match parse_address(&address)? {
                ForceAddr::Bit { area, byte_addr, bit } => {
                    let v = parse_bit_value(&value)?;
                    client.force_bit(area, byte_addr, bit, v).await?;
                    let area_name = area_char(area);
                    println!(
                        "ok  – forced {}{}.{} = {}",
                        area_name, byte_addr, bit, if v { 1 } else { 0 }
                    );
                }
                ForceAddr::Byte { area, byte_addr } => {
                    let v = parse_byte_value(&value)?;
                    client.force_byte(area, byte_addr, v).await?;
                    let area_name = area_byte_prefix(area);
                    println!("ok  – forced {}{} = 0x{:02X}", area_name, byte_addr, v);
                }
            }
        }
        ForceAction::Cancel { address } => {
            let addr = parse_byte_address(&address)?;
            client.force_cancel_byte(addr.area, addr.byte_addr).await?;
            let area_name = area_byte_prefix(addr.area);
            println!("ok  – force cancelled on {}{}", area_name, addr.byte_addr);
        }
        ForceAction::List => {
            let data = client.read_force_list().await?;
            if data.is_empty() {
                println!("no forced variables");
                return Ok(());
            }
            print_force_list(&data);
        }
    }
    Ok(())
}

enum ForceAddr {
    Bit { area: Area, byte_addr: u32, bit: u8 },
    Byte { area: Area, byte_addr: u32 },
}

struct ByteAddr {
    area: Area,
    byte_addr: u32,
}

fn parse_address(s: &str) -> Result<ForceAddr> {
    let upper = s.to_uppercase();
    // Byte forms: IB<n>, QB<n>
    if upper.starts_with("IB") {
        let n: u32 = upper[2..].parse().map_err(|_| anyhow::anyhow!("invalid byte address: {s}"))?;
        return Ok(ForceAddr::Byte { area: Area::ProcessInput, byte_addr: n });
    }
    if upper.starts_with("QB") {
        let n: u32 = upper[2..].parse().map_err(|_| anyhow::anyhow!("invalid byte address: {s}"))?;
        return Ok(ForceAddr::Byte { area: Area::ProcessOutput, byte_addr: n });
    }
    // Bit forms: I<n>.<b>, Q<n>.<b>
    let (area, rest) = if upper.starts_with('I') {
        (Area::ProcessInput, &upper[1..])
    } else if upper.starts_with('Q') {
        (Area::ProcessOutput, &upper[1..])
    } else {
        bail!("address must start with I, Q, IB, or QB — got: {s}");
    };
    let parts: Vec<&str> = rest.splitn(2, '.').collect();
    if parts.len() != 2 {
        bail!("bit address requires dot notation: I<byte>.<bit> or Q<byte>.<bit>");
    }
    let byte_addr: u32 = parts[0].parse().map_err(|_| anyhow::anyhow!("invalid byte in: {s}"))?;
    let bit: u8 = parts[1].parse().map_err(|_| anyhow::anyhow!("invalid bit in: {s}"))?;
    if bit > 7 {
        bail!("bit must be 0–7, got {bit}");
    }
    Ok(ForceAddr::Bit { area, byte_addr, bit })
}

fn parse_byte_address(s: &str) -> Result<ByteAddr> {
    let upper = s.to_uppercase();
    if upper.starts_with("IB") {
        let n: u32 = upper[2..].parse().map_err(|_| anyhow::anyhow!("invalid address: {s}"))?;
        return Ok(ByteAddr { area: Area::ProcessInput, byte_addr: n });
    }
    if upper.starts_with("QB") {
        let n: u32 = upper[2..].parse().map_err(|_| anyhow::anyhow!("invalid address: {s}"))?;
        return Ok(ByteAddr { area: Area::ProcessOutput, byte_addr: n });
    }
    bail!("cancel address must be IB<n> or QB<n> — got: {s}");
}

fn parse_bit_value(s: &str) -> Result<bool> {
    match s {
        "0" | "false" | "False" | "FALSE" => Ok(false),
        "1" | "true" | "True" | "TRUE" => Ok(true),
        _ => bail!("bit value must be 0 or 1, got: {s}"),
    }
}

fn parse_byte_value(s: &str) -> Result<u8> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        u8::from_str_radix(&s[2..], 16).map_err(|_| anyhow::anyhow!("invalid hex byte: {s}"))
    } else {
        s.parse::<u8>().map_err(|_| anyhow::anyhow!("invalid byte value (0–255 or 0x00–0xFF): {s}"))
    }
}

fn area_char(area: Area) -> &'static str {
    match area {
        Area::ProcessInput => "I",
        Area::ProcessOutput => "Q",
        _ => "?",
    }
}

fn area_byte_prefix(area: Area) -> &'static str {
    match area {
        Area::ProcessInput => "IB",
        Area::ProcessOutput => "QB",
        _ => "?B",
    }
}

fn print_force_list(data: &bytes::Bytes) {
    // SZL block layout: szl_id(2) + szl_index(2) + entry_len(2) + entry_count(2) + entries
    if data.len() < 8 {
        println!("force list: <too short to parse>");
        return;
    }
    let entry_len = u16::from_be_bytes([data[4], data[5]]) as usize;
    let entry_count = u16::from_be_bytes([data[6], data[7]]) as usize;
    if entry_len == 0 || entry_count == 0 {
        println!("no forced variables");
        return;
    }
    println!("{} forced variable(s):", entry_count);
    for i in 0..entry_count {
        let off = 8 + i * entry_len;
        if off + entry_len > data.len() {
            break;
        }
        let entry = &data[off..off + entry_len];
        println!("  [{:2}] {:02X?}", i, entry);
    }
}
