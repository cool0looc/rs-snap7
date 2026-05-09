use std::io::Write;
use anyhow::{bail, Result};
use snap7_client::{transport::TcpTransport, types::{BlockAttributes, BlockData}, S7Client};

use crate::args::{BlockAction, BlockArgs};

pub async fn run(client: &S7Client<TcpTransport>, args: BlockArgs) -> Result<()> {
    match args.action {
        BlockAction::List => {
            let list = client.list_blocks().await?;
            println!("Total blocks: {}", list.total_count);
            if list.entries.is_empty() {
                println!("(no block type entries)");
            }
            for e in &list.entries {
                let label = block_type_label(e.block_type);
                println!("  {} (0x{:04X}): {} blocks", label, e.block_type, e.count);
            }
        }
        BlockAction::Numbers { r#type } => {
            let bt = parse_block_type(&r#type)?;
            let nums = client.list_blocks_of_type(bt).await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("{} blocks of type {}:", nums.len(), r#type.to_uppercase());
            for n in &nums {
                println!("  {}{}", r#type.to_uppercase(), n);
            }
        }
        BlockAction::Info { r#type, number } => {
            let bt = parse_block_type(&r#type)?;
            let info = client.get_ag_block_info(bt, number).await?;
            println!("Block type:   0x{:04X} ({})", info.block_type, r#type);
            println!("Block number: {}", info.block_number);
            println!("Language:     {}", info.language);
            println!("Flags:        0x{:04X}", info.flags);
            println!("Size:         {} bytes", info.size);
            println!("RAM size:     {} bytes", info.size_ram);
            println!("MC7 size:     {} bytes", info.mc7_size);
            println!("Local data:   {} bytes", info.local_data);
            println!("Checksum:     0x{:04X}", info.checksum);
            println!("Version:      0x{:04X}", info.version);
            println!("Author:       {}", info.author);
            println!("Family:       {}", info.family);
            println!("Header:       {}", info.header);
            println!("Date:         {}", info.date);
        }
        BlockAction::Upload { r#type, number, out } => {
            let bt = parse_block_type(&r#type)?;
            let data = client.upload(bt, number).await?;
            let mut file = std::fs::File::create(&out)?;
            file.write_all(&data)?;
            if let Some(bd) = BlockData::from_bytes(&data) {
                println!(
                    "Uploaded {} {} ({} bytes + {} payload) → {}",
                    r#type, number, bd.total_length, bd.payload.len(), out
                );
            } else {
                println!("Uploaded {} {} ({} raw bytes) → {}", r#type, number, data.len(), out);
            }
        }
        BlockAction::Download { r#type, number, file } => {
            let bt = parse_block_type(&r#type)?;
            let data = std::fs::read(&file)?;
            client.download(bt, number, &data).await?;
            println!("ok  – downloaded {} {} from {}", r#type.to_uppercase(), number, file);
        }
        BlockAction::CreateDb { number, size, author, family, name, version } => {
            let attrs = BlockAttributes {
                author,
                family,
                name,
                version: parse_version_opt(version.as_deref())?,
                flags: None,
            };
            client.create_db(number, size, Some(&attrs)).await?;
            println!("ok  – created DB{} ({} bytes)", number, size);
        }
        BlockAction::SetAttrs { r#type, number, author, family, name, version } => {
            let bt = parse_block_type(&r#type)?;
            // Upload, modify, re-download
            let raw = client.full_upload(bt, number).await?;
            let mut block = BlockData::from_bytes(&raw)
                .ok_or_else(|| anyhow::anyhow!("failed to parse block data"))?;
            let attrs = BlockAttributes {
                author,
                family,
                name,
                version: parse_version_opt(version.as_deref())?,
                flags: None,
            };
            block.set_attributes(&attrs);
            let bytes = block.to_bytes();
            client.download(bt, number, &bytes).await?;
            println!("ok  – attributes updated on {}{}", r#type.to_uppercase(), number);
        }
    }
    Ok(())
}

fn block_type_label(t: u16) -> &'static str {
    match t {
        0x38 => "OB",
        0x41 => "DB",
        0x42 => "SDB",
        0x43 => "FC",
        0x44 => "SFC",
        0x45 => "FB",
        0x46 => "SFB",
        0x47 => "DI",
        _ => "?",
    }
}

pub fn parse_block_type(s: &str) -> Result<u8> {
    Ok(match s.to_uppercase().as_str() {
        "OB" => 0x38,
        "DB" => 0x41,
        "SDB" => 0x42,
        "FC" => 0x43,
        "SFC" => 0x44,
        "FB" => 0x45,
        "SFB" => 0x46,
        "UDT" => 0x47,
        _ => bail!("unknown block type: {}. Use OB, DB, FC, FB, SFC, SFB, SDB, UDT", s),
    })
}

fn parse_version_opt(s: Option<&str>) -> Result<Option<u8>> {
    let s = match s {
        Some(s) => s,
        None => return Ok(None),
    };
    let parts: Vec<&str> = s.splitn(2, '.').collect();
    let major: u8 = parts[0].parse().map_err(|_| anyhow::anyhow!("invalid version major: {s}"))?;
    let minor: u8 = if parts.len() > 1 {
        parts[1].parse().map_err(|_| anyhow::anyhow!("invalid version minor: {s}"))?
    } else { 0 };
    if major > 15 || minor > 15 {
        bail!("version major and minor must each be 0–15");
    }
    Ok(Some((major << 4) | minor))
}
