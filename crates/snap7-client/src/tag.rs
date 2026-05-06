use crate::proto::s7::header::{Area, TransportSize};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct TagAddress {
    pub area: Area,
    pub db_number: u16,
    pub byte_offset: u32,
    pub bit_offset: u8,
    pub transport: TransportSize,
    pub element_count: u16,
}

pub fn parse_tag(tag: &str) -> Result<TagAddress> {
    let parts: Vec<&str> = tag.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(Error::PlcError {
            code: 0,
            message: format!("invalid tag: {}", tag),
        });
    }
    let area_part = parts[0].to_uppercase();
    let type_part = parts[1].to_uppercase();

    let (area, db_number) = if let Some(rest) = area_part.strip_prefix("DB") {
        let n: u16 = rest.parse().map_err(|_| Error::PlcError {
            code: 0,
            message: format!("invalid DB number in tag: {}", tag),
        })?;
        (Area::DataBlock, n)
    } else {
        return Err(Error::PlcError {
            code: 0,
            message: format!("unsupported area in tag: {}", tag),
        });
    };

    // Support bit access format: DB70,332.0 (byte 332, bit 0)
    // type_part is already uppercased from line 24
    // For bit access, type_part starts with a digit (byte offset)
    if type_part.starts_with(|c: char| c.is_ascii_digit()) {
        let bits: Vec<&str> = type_part.split('.').collect();
        if bits.len() == 2 {
            let byte_offset: u32 = bits[0].parse().map_err(|_| Error::PlcError {
                code: 0,
                message: format!("invalid byte offset in tag: {}", tag),
            })?;
            let bit_offset: u8 = bits[1].parse().map_err(|_| Error::PlcError {
                code: 0,
                message: format!("invalid bit offset in tag: {}", tag),
            })?;
            if bit_offset > 7 {
                return Err(Error::PlcError {
                    code: 0,
                    message: format!("bit offset must be 0-7: {}", tag),
                });
            }
            return Ok(TagAddress {
                area,
                db_number,
                byte_offset,
                bit_offset,
                transport: TransportSize::Bit,
                element_count: 1,
            });
        }
    }

    let (transport, byte_offset) = if let Some(rest) = type_part.strip_prefix("REAL") {
        (TransportSize::Real, rest.parse().unwrap_or(0))
    } else if let Some(rest) = type_part.strip_prefix("DWORD") {
        (TransportSize::DWord, rest.parse().unwrap_or(0))
    } else if let Some(rest) = type_part.strip_prefix("DINT") {
        (TransportSize::DInt, rest.parse().unwrap_or(0))
    } else if let Some(rest) = type_part.strip_prefix("WORD") {
        (TransportSize::Word, rest.parse().unwrap_or(0))
    } else if let Some(rest) = type_part.strip_prefix("INT") {
        (TransportSize::Int, rest.parse().unwrap_or(0))
    } else if let Some(rest) = type_part.strip_prefix("BYTE") {
        (TransportSize::Byte, rest.parse().unwrap_or(0))
    } else {
        return Err(Error::PlcError {
            code: 0,
            message: format!("unsupported type in tag: {}", tag),
        });
    };

    Ok(TagAddress {
        area,
        db_number,
        byte_offset,
        bit_offset: 0,
        transport,
        element_count: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_db_real() {
        let tag = parse_tag("DB1,REAL4").unwrap();
        assert_eq!(tag.db_number, 1);
        assert_eq!(tag.byte_offset, 4);
        assert_eq!(tag.transport, TransportSize::Real);
    }

    #[test]
    fn parse_db_word() {
        let tag = parse_tag("DB2,WORD10").unwrap();
        assert_eq!(tag.db_number, 2);
        assert_eq!(tag.byte_offset, 10);
        assert_eq!(tag.transport, TransportSize::Word);
    }

    #[test]
    fn parse_db_dint() {
        let tag = parse_tag("DB70,DINT0").unwrap();
        assert_eq!(tag.db_number, 70);
        assert_eq!(tag.byte_offset, 0);
        assert_eq!(tag.transport, TransportSize::DInt);
        assert_eq!(tag.bit_offset, 0);
    }

    #[test]
    fn parse_db_bit_access() {
        let tag = parse_tag("DB70,332.0").unwrap();
        assert_eq!(tag.db_number, 70);
        assert_eq!(tag.byte_offset, 332);
        assert_eq!(tag.bit_offset, 0);
        assert_eq!(tag.transport, TransportSize::Bit);
    }

    #[test]
    fn parse_db_bit_access_bit7() {
        let tag = parse_tag("DB70,332.7").unwrap();
        assert_eq!(tag.db_number, 70);
        assert_eq!(tag.byte_offset, 332);
        assert_eq!(tag.bit_offset, 7);
        assert_eq!(tag.transport, TransportSize::Bit);
    }

    #[test]
    fn parse_db_bit_invalid_bit() {
        let result = parse_tag("DB70,332.8");
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_returns_err() {
        assert!(parse_tag("NOTVALID").is_err());
    }
}
