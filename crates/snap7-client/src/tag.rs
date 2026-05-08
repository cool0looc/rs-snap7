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
    let upper = tag.to_uppercase();

    // Single-part tags: T<n>, C<n>, MB<n>, MW<n>, MD<n>, MX<n>.<b>, M<n>.<b>
    // These do not use a comma separator.
    if upper.starts_with('T') && !upper.starts_with("TM") && !upper.contains(',') {
        let idx_str = upper.strip_prefix('T').unwrap_or("");
        let index: u32 = idx_str.parse().map_err(|_| Error::PlcError {
            code: 0,
            message: format!("invalid timer index in tag: {}", tag),
        })?;
        return Ok(TagAddress {
            area: Area::Timer,
            db_number: 0,
            byte_offset: index,
            bit_offset: 0,
            transport: TransportSize::Timer,
            element_count: 1,
        });
    }
    if upper.starts_with('C') && !upper.starts_with("CT") && !upper.contains(',') {
        let idx_str = upper.strip_prefix('C').unwrap_or("");
        let index: u32 = idx_str.parse().map_err(|_| Error::PlcError {
            code: 0,
            message: format!("invalid counter index in tag: {}", tag),
        })?;
        return Ok(TagAddress {
            area: Area::Counter,
            db_number: 0,
            byte_offset: index,
            bit_offset: 0,
            transport: TransportSize::Counter,
            element_count: 1,
        });
    }
    if upper.starts_with('M') && !upper.starts_with("MK") && !upper.contains(',') {
        return parse_marker_tag(&upper, "", tag);
    }

    // Accept both "DB170,REAL262" and "DB170.REAL262" as separators.
    // The dot separator is only valid between the area (DB\d+) and the type;
    // normalize it to a comma so the rest of the parser is uniform.
    let normalized: std::borrow::Cow<str> = if tag.contains(',') {
        std::borrow::Cow::Borrowed(tag)
    } else {
        // Find first '.' that is preceded by a digit (end of DB number) and
        // followed by a letter (start of type name), then replace it with ','.
        // Find pattern: digit '.' letter  e.g. "170.REAL" → replace '.' with ','
        let bytes = tag.as_bytes();
        let sep = bytes.windows(3).enumerate().find_map(|(i, w)| {
            if w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_alphabetic() {
                Some(i + 1) // position of the '.'
            } else {
                None
            }
        });
        if let Some(pos) = sep {
            let mut s = tag.to_string();
            s.replace_range(pos..pos + 1, ",");
            std::borrow::Cow::Owned(s)
        } else {
            std::borrow::Cow::Borrowed(tag)
        }
    };

    let parts: Vec<&str> = normalized.splitn(2, ',').collect();
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

    let (transport, byte_offset) = parse_typed_offset(&type_part, tag)?;

    Ok(TagAddress {
        area,
        db_number,
        byte_offset,
        bit_offset: 0,
        transport,
        element_count: 1,
    })
}

/// Parse a Merker (M) tag. area_part and type_part are already uppercased.
/// Formats:
///   MX10.3  or  M10.3  → bit access (byte 10, bit 3)
///   MB10              → byte at offset 10
///   MW10              → word at offset 10
///   MD10              → dword at offset 10
fn parse_marker_tag(area_part: &str, _type_part: &str, tag: &str) -> Result<TagAddress> {
    // area_part already uppercased; strip leading "M"
    let rest = area_part.strip_prefix('M').unwrap_or("");

    // MX or plain M followed by digit.digit → bit
    let (is_bit, offset_str) = if let Some(r) = rest.strip_prefix('X') {
        (true, r)
    } else if rest.starts_with(|c: char| c.is_ascii_digit()) {
        (true, rest)
    } else {
        (false, rest)
    };

    if is_bit {
        // expect "BYTE.BIT" e.g. "10.3"
        let parts: Vec<&str> = offset_str.split('.').collect();
        if parts.len() == 2 {
            let byte_offset: u32 = parts[0].parse().map_err(|_| Error::PlcError {
                code: 0,
                message: format!("invalid byte offset in marker tag: {}", tag),
            })?;
            let bit_offset: u8 = parts[1].parse().map_err(|_| Error::PlcError {
                code: 0,
                message: format!("invalid bit offset in marker tag: {}", tag),
            })?;
            if bit_offset > 7 {
                return Err(Error::PlcError {
                    code: 0,
                    message: format!("bit offset must be 0-7: {}", tag),
                });
            }
            return Ok(TagAddress {
                area: Area::Marker,
                db_number: 0,
                byte_offset,
                bit_offset,
                transport: TransportSize::Bit,
                element_count: 1,
            });
        }
        return Err(Error::PlcError {
            code: 0,
            message: format!("invalid marker bit tag (expected M<byte>.<bit>): {}", tag),
        });
    }

    // MB / MW / MD
    let (transport, offset_str) = if let Some(r) = rest.strip_prefix('B') {
        (TransportSize::Byte, r)
    } else if let Some(r) = rest.strip_prefix('W') {
        (TransportSize::Word, r)
    } else if let Some(r) = rest.strip_prefix('D') {
        (TransportSize::DWord, r)
    } else {
        return Err(Error::PlcError {
            code: 0,
            message: format!("unsupported marker type in tag: {} (use MB/MW/MD/MX or M<byte>.<bit>)", tag),
        });
    };

    let byte_offset: u32 = offset_str.parse().map_err(|_| Error::PlcError {
        code: 0,
        message: format!("invalid offset in marker tag: {}", tag),
    })?;

    Ok(TagAddress {
        area: Area::Marker,
        db_number: 0,
        byte_offset,
        bit_offset: 0,
        transport,
        element_count: 1,
    })
}

fn parse_typed_offset(type_part: &str, tag: &str) -> Result<(TransportSize, u32)> {
    if let Some(rest) = type_part.strip_prefix("REAL") {
        Ok((TransportSize::Real, rest.parse().unwrap_or(0)))
    } else if let Some(rest) = type_part.strip_prefix("DWORD") {
        Ok((TransportSize::DWord, rest.parse().unwrap_or(0)))
    } else if let Some(rest) = type_part.strip_prefix("DINT") {
        Ok((TransportSize::DInt, rest.parse().unwrap_or(0)))
    } else if let Some(rest) = type_part.strip_prefix("WORD") {
        Ok((TransportSize::Word, rest.parse().unwrap_or(0)))
    } else if let Some(rest) = type_part.strip_prefix("INT") {
        Ok((TransportSize::Int, rest.parse().unwrap_or(0)))
    } else if let Some(rest) = type_part.strip_prefix("BYTE") {
        Ok((TransportSize::Byte, rest.parse().unwrap_or(0)))
    } else {
        Err(Error::PlcError {
            code: 0,
            message: format!("unsupported type in tag: {}", tag),
        })
    }
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

    #[test]
    fn parse_dot_separator_real() {
        let tag = parse_tag("DB170.REAL262").unwrap();
        assert_eq!(tag.db_number, 170);
        assert_eq!(tag.byte_offset, 262);
        assert_eq!(tag.transport, TransportSize::Real);
    }

    #[test]
    fn parse_dot_separator_word() {
        let tag = parse_tag("DB1.WORD10").unwrap();
        assert_eq!(tag.db_number, 1);
        assert_eq!(tag.byte_offset, 10);
        assert_eq!(tag.transport, TransportSize::Word);
    }

    #[test]
    fn parse_comma_separator_unchanged() {
        let a = parse_tag("DB170,REAL262").unwrap();
        let b = parse_tag("DB170.REAL262").unwrap();
        assert_eq!(a.db_number, b.db_number);
        assert_eq!(a.byte_offset, b.byte_offset);
        assert_eq!(a.transport, b.transport);
    }

    #[test]
    fn parse_bit_access_dot_not_confused() {
        // DB70,332.0 — the comma is already there, dot is bit separator
        let tag = parse_tag("DB70,332.0").unwrap();
        assert_eq!(tag.transport, TransportSize::Bit);
        assert_eq!(tag.byte_offset, 332);
        assert_eq!(tag.bit_offset, 0);
    }

    #[test]
    fn parse_timer_single_part() {
        let tag = parse_tag("T5").unwrap();
        assert_eq!(tag.area, Area::Timer);
        assert_eq!(tag.byte_offset, 5);
        assert_eq!(tag.transport, TransportSize::Timer);
        assert_eq!(tag.db_number, 0);
    }

    #[test]
    fn parse_counter_single_part() {
        let tag = parse_tag("C3").unwrap();
        assert_eq!(tag.area, Area::Counter);
        assert_eq!(tag.byte_offset, 3);
        assert_eq!(tag.transport, TransportSize::Counter);
        assert_eq!(tag.db_number, 0);
    }

    #[test]
    fn parse_marker_byte_single_part() {
        let tag = parse_tag("MB10").unwrap();
        assert_eq!(tag.area, Area::Marker);
        assert_eq!(tag.byte_offset, 10);
        assert_eq!(tag.transport, TransportSize::Byte);
    }

    #[test]
    fn parse_marker_word_single_part() {
        let tag = parse_tag("MW20").unwrap();
        assert_eq!(tag.area, Area::Marker);
        assert_eq!(tag.byte_offset, 20);
        assert_eq!(tag.transport, TransportSize::Word);
    }

    #[test]
    fn parse_marker_dword_single_part() {
        let tag = parse_tag("MD4").unwrap();
        assert_eq!(tag.area, Area::Marker);
        assert_eq!(tag.byte_offset, 4);
        assert_eq!(tag.transport, TransportSize::DWord);
    }

    #[test]
    fn parse_marker_bit_single_part() {
        let tag = parse_tag("M10.3").unwrap();
        assert_eq!(tag.area, Area::Marker);
        assert_eq!(tag.byte_offset, 10);
        assert_eq!(tag.bit_offset, 3);
        assert_eq!(tag.transport, TransportSize::Bit);
    }

    #[test]
    fn parse_marker_bit_mx_prefix() {
        let tag = parse_tag("MX5.7").unwrap();
        assert_eq!(tag.area, Area::Marker);
        assert_eq!(tag.byte_offset, 5);
        assert_eq!(tag.bit_offset, 7);
        assert_eq!(tag.transport, TransportSize::Bit);
    }
}
