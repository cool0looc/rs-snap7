use bytes::Bytes;

use crate::args::OutputFormat;

pub fn print_bytes(data: &Bytes, format: &OutputFormat) {
    match format {
        OutputFormat::Text => {
            for (i, chunk) in data.chunks(16).enumerate() {
                let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
                let ascii: String = chunk
                    .iter()
                    .map(|b| {
                        if b.is_ascii_graphic() {
                            *b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                println!("{:04X}  {:47}  {}", i * 16, hex.join(" "), ascii);
            }
        }
        OutputFormat::Json => {
            let hex: Vec<String> = data.iter().map(|b| format!("{:02X}", b)).collect();
            println!(
                "{{\"bytes\":[{}]}}",
                hex.iter()
                    .map(|h| format!("\"0x{}\"", h))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        OutputFormat::Csv => {
            println!("offset,value");
            for (i, b) in data.iter().enumerate() {
                println!("{},{:02X}", i, b);
            }
        }
    }
}
