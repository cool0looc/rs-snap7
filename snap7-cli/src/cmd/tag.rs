use anyhow::Result;
use bytes::Bytes;
use snap7_client::proto::s7::header::{Area, TransportSize};
use snap7_client::{tag::parse_tag, transport::TcpTransport, S7Client};

use crate::args::{OutputFormat, TagAction, TagArgs};

pub async fn run(
    client: &S7Client<TcpTransport>,
    args: TagArgs,
    _format: &OutputFormat,
) -> Result<()> {
    match args.action {
        TagAction::Read { tag } => {
            let addr = parse_tag(&tag).map_err(|e| anyhow::anyhow!("{}", e))?;
            let data = read_tag(client, &addr).await?;
            println!("{}", decode_value(&data, addr.transport));
        }
        TagAction::Write { tag, value } => {
            let addr = parse_tag(&tag).map_err(|e| anyhow::anyhow!("{}", e))?;
            let data = encode_value(&value, addr.transport)?;
            write_tag(client, &addr, &data).await?;
            println!("ok");
        }
    }
    Ok(())
}

async fn read_tag(
    client: &S7Client<snap7_client::transport::TcpTransport>,
    addr: &snap7_client::tag::TagAddress,
) -> Result<Bytes> {
    use snap7_client::proto::s7::header::Area::*;
    match addr.area {
        DataBlock => client
            .db_read(addr.db_number, addr.byte_offset, transport_size_bytes(addr.transport))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e)),
        Timer | Counter | Marker | ProcessInput | ProcessOutput | PeripheralInput | InstanceDB | LocalData => client
            .read_area(
                addr.area,
                addr.db_number,
                addr.byte_offset,
                addr.element_count,
                addr.transport,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{}", e)),
    }
}

async fn write_tag(
    client: &S7Client<snap7_client::transport::TcpTransport>,
    addr: &snap7_client::tag::TagAddress,
    data: &[u8],
) -> Result<()> {
    match addr.area {
        Area::DataBlock => client
            .db_write(addr.db_number, addr.byte_offset, data)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e)),
        _ => client
            .write_area(addr.area, addr.db_number, addr.byte_offset, addr.transport, data)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e)),
    }
}

fn transport_size_bytes(ts: TransportSize) -> u16 {
    match ts {
        TransportSize::Bit | TransportSize::Byte | TransportSize::Char => 1,
        TransportSize::Word
        | TransportSize::Int
        | TransportSize::S5Time
        | TransportSize::Date
        | TransportSize::Timer
        | TransportSize::Counter => 2,
        TransportSize::DWord
        | TransportSize::DInt
        | TransportSize::Real
        | TransportSize::Time
        | TransportSize::Tod => 4,
        TransportSize::DtL => 12,
    }
}

fn decode_value(data: &Bytes, ts: TransportSize) -> String {
    match ts {
        TransportSize::Real if data.len() >= 4 => {
            format!(
                "{}",
                f32::from_be_bytes([data[0], data[1], data[2], data[3]])
            )
        }
        TransportSize::Word if data.len() >= 2 => {
            format!("{}", u16::from_be_bytes([data[0], data[1]]))
        }
        TransportSize::DWord if data.len() >= 4 => {
            format!(
                "{}",
                u32::from_be_bytes([data[0], data[1], data[2], data[3]])
            )
        }
        TransportSize::Int if data.len() >= 2 => {
            format!("{}", i16::from_be_bytes([data[0], data[1]]))
        }
        TransportSize::DInt if data.len() >= 4 => {
            format!(
                "{}",
                i32::from_be_bytes([data[0], data[1], data[2], data[3]])
            )
        }
        TransportSize::Byte if !data.is_empty() => format!("{}", data[0]),
        _ => data
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn encode_value(value: &str, ts: TransportSize) -> Result<Vec<u8>> {
    Ok(match ts {
        TransportSize::Real => {
            let v: f32 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid float: {}", value))?;
            v.to_be_bytes().to_vec()
        }
        TransportSize::Word => {
            let v: u16 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid u16: {}", value))?;
            v.to_be_bytes().to_vec()
        }
        TransportSize::DWord => {
            let v: u32 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid u32: {}", value))?;
            v.to_be_bytes().to_vec()
        }
        TransportSize::Int => {
            let v: i16 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid i16: {}", value))?;
            v.to_be_bytes().to_vec()
        }
        TransportSize::DInt => {
            let v: i32 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid i32: {}", value))?;
            v.to_be_bytes().to_vec()
        }
        TransportSize::Byte => {
            let v: u8 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid u8: {}", value))?;
            vec![v]
        }
        _ => return Err(anyhow::anyhow!("unsupported type for write: {:?}", ts)),
    })
}
