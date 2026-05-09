use anyhow::{bail, Result};
use snap7_client::{block_type_name, types::BlockCmpResult, transport::TcpTransport, S7Client};
use std::io::Write;

use crate::args::{ProgramAction, ProgramArgs};
use crate::cmd::block::parse_block_type;

pub async fn run(client: &S7Client<TcpTransport>, args: ProgramArgs) -> Result<()> {
    match args.action {
        ProgramAction::MemReset { force } => {
            if !force {
                confirm_destructive("perform memory reset (clears all work memory blocks)")?;
            }
            client.memory_reset().await?;
            println!("ok  – memory reset complete");
        }
        ProgramAction::Format { force } => {
            if !force {
                confirm_destructive(
                    "perform OVERALL RESET (wipes all load memory, work memory, and retain data)",
                )?;
            }
            client.overall_reset().await?;
            println!("ok  – overall reset complete");
        }
        ProgramAction::BatchUpload { types, out, full } => {
            let block_types = parse_types_list(&types)?;
            let out_dir = std::path::Path::new(&out);
            std::fs::create_dir_all(out_dir)?;

            let mut total = 0usize;
            for bt in &block_types {
                let numbers = client.list_blocks_of_type(*bt).await?;
                for num in numbers {
                    let data = if full {
                        client.full_upload(*bt, num).await?
                    } else {
                        client.upload(*bt, num).await?
                    };
                    let fname = format!("{}{}.bin", block_type_name(*bt), num);
                    let path = out_dir.join(&fname);
                    std::fs::File::create(&path)?.write_all(&data)?;
                    eprintln!("  {} → {}", fname, path.display());
                    total += 1;
                }
            }
            println!("ok  – uploaded {} blocks to {}", total, out);
        }
        ProgramAction::Compare { dir, plc_only } => {
            let dir_path = std::path::Path::new(&dir);
            if !dir_path.is_dir() {
                bail!("not a directory: {}", dir);
            }

            // Load local blocks: files named <TYPE><NUM>.bin
            let mut local: Vec<(u8, u16, Vec<u8>)> = Vec::new();
            for entry in std::fs::read_dir(dir_path)? {
                let entry = entry?;
                let fname = entry.file_name().to_string_lossy().to_string();
                if !fname.ends_with(".bin") {
                    continue;
                }
                if let Some((bt, num)) = parse_block_filename(&fname) {
                    let data = std::fs::read(entry.path())?;
                    local.push((bt, num, data));
                }
            }

            if local.is_empty() {
                println!("no .bin block files found in {}", dir);
                return Ok(());
            }

            let results = client.compare_blocks(&local, plc_only).await?;

            let mut matches = 0;
            let mut mismatches = 0;
            let mut only_local = 0;
            let mut only_plc = 0;

            for (bt, num, result) in &results {
                let label = format!("{}{}", block_type_name(*bt), num);
                match result {
                    BlockCmpResult::Match => {
                        println!("  ✓  {}", label);
                        matches += 1;
                    }
                    BlockCmpResult::Mismatch { local_crc, plc_crc } => {
                        println!(
                            "  ✗  {} — MISMATCH (local=0x{:08X} plc=0x{:08X})",
                            label, local_crc, plc_crc
                        );
                        mismatches += 1;
                    }
                    BlockCmpResult::OnlyLocal => {
                        println!("  ?  {} — only in local (not on PLC)", label);
                        only_local += 1;
                    }
                    BlockCmpResult::OnlyPlc => {
                        println!("  +  {} — only on PLC (not in local)", label);
                        only_plc += 1;
                    }
                }
            }

            println!(
                "\n{} match  {} mismatch  {} local-only  {} plc-only",
                matches, mismatches, only_local, only_plc
            );
        }
    }
    Ok(())
}

fn parse_types_list(s: &str) -> Result<Vec<u8>> {
    s.split(',')
        .map(|t| parse_block_type(t.trim()))
        .collect()
}

fn parse_block_filename(fname: &str) -> Option<(u8, u16)> {
    // Expect: OB1.bin, DB100.bin, FC5.bin, FB10.bin, etc.
    let base = fname.strip_suffix(".bin")?;
    for prefix_len in [3, 2] {
        if base.len() <= prefix_len {
            continue;
        }
        let (type_str, num_str) = base.split_at(prefix_len);
        if let (Ok(bt), Ok(num)) = (parse_block_type(type_str), num_str.parse::<u16>()) {
            return Some((bt, num));
        }
    }
    None
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
