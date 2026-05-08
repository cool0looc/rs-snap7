use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::proto::error::ProtoError;
use crate::proto::s7::header::TransportSize;
use crate::proto::s7::read_var::{AddressItem, ADDR_ANY, ITEM_LEN, ITEM_SPEC};

pub const FUNC_WRITE_VAR: u8 = 0x05;

#[derive(Debug, Clone)]
pub struct WriteItem {
    pub address: AddressItem,
    pub data: Bytes,
}

#[derive(Debug, Clone)]
pub struct WriteVarRequest {
    pub items: Vec<WriteItem>,
}

#[derive(Debug, Clone)]
pub struct WriteVarResponse {
    pub return_codes: Vec<u8>,
}

impl WriteVarRequest {
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        use crate::proto::s7::header::{Area, TransportSize};

        if buf.len() < 2 {
            return Err(ProtoError::BufferTooShort {
                need: 2,
                have: buf.len(),
            });
        }
        let func = buf.get_u8();
        if func != FUNC_WRITE_VAR {
            return Err(ProtoError::UnsupportedFunction(func));
        }
        let count = buf.get_u8() as usize;

        // Decode address items
        let mut addresses = Vec::with_capacity(count);
        for _ in 0..count {
            if buf.len() < 12 {
                return Err(ProtoError::BufferTooShort {
                    need: 12,
                    have: buf.len(),
                });
            }
            let spec = buf.get_u8();
            if spec != ITEM_SPEC {
                return Err(ProtoError::InvalidMagic {
                    expected: ITEM_SPEC,
                    got: spec,
                });
            }
            buf.get_u8(); // ITEM_LEN (reserved)
            let addr_type = buf.get_u8();
            if addr_type != ADDR_ANY {
                return Err(ProtoError::InvalidMagic {
                    expected: ADDR_ANY,
                    got: addr_type,
                });
            }
            let transport = TransportSize::try_from(buf.get_u8())?;
            let length = buf.get_u16();
            let db_number = buf.get_u16();
            let area = Area::try_from(buf.get_u8())?;
            let b0 = buf.get_u8() as u32;
            let b1 = buf.get_u8() as u32;
            let b2 = buf.get_u8() as u32;
            let addr_bits = (b0 << 16) | (b1 << 8) | b2;
            let start = addr_bits >> 3;
            let bit_offset = (addr_bits & 0x07) as u8;
            addresses.push(AddressItem {
                area,
                db_number,
                start,
                bit_offset,
                length,
                transport,
            });
        }

        // Decode data items
        let mut items = Vec::with_capacity(count);
        for address in addresses {
            if buf.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    need: 4,
                    have: buf.len(),
                });
            }
            buf.get_u8(); // reserved
            buf.get_u8(); // transport size
            let bit_len = buf.get_u16() as usize;
            let byte_len = bit_len.div_ceil(8);
            if buf.len() < byte_len {
                return Err(ProtoError::BufferTooShort {
                    need: byte_len,
                    have: buf.len(),
                });
            }
            let data = buf.copy_to_bytes(byte_len);
            // consume pad byte if odd length
            if !byte_len.is_multiple_of(2) && buf.has_remaining() {
                buf.advance(1);
            }
            items.push(WriteItem { address, data });
        }

        Ok(WriteVarRequest { items })
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(FUNC_WRITE_VAR);
        buf.put_u8(self.items.len() as u8);
        // address items first
        for item in &self.items {
            let addr = &item.address;
            buf.put_u8(ITEM_SPEC);
            buf.put_u8(ITEM_LEN);
            buf.put_u8(ADDR_ANY);
            buf.put_u8(addr.transport as u8);
            buf.put_u16(addr.length);
            buf.put_u16(addr.db_number);
            buf.put_u8(addr.area as u8);
            let addr_bits = match addr.transport {
                TransportSize::Timer | TransportSize::Counter => addr.start,
                _ => (addr.start * 8) | (addr.bit_offset as u32),
            };
            buf.put_u8(((addr_bits >> 16) & 0xFF) as u8);
            buf.put_u8(((addr_bits >> 8) & 0xFF) as u8);
            buf.put_u8((addr_bits & 0xFF) as u8);
        }
        // then data items
        for item in &self.items {
            buf.put_u8(0x00); // reserved
            buf.put_u8(item.address.transport as u8);
            let bit_len = (item.data.len() * 8) as u16;
            buf.put_u16(bit_len);
            buf.put_slice(&item.data);
            if !item.data.len().is_multiple_of(2) {
                buf.put_u8(0x00); // pad to even boundary
            }
        }
    }
}

impl WriteVarResponse {
    pub fn decode(buf: &mut Bytes, item_count: usize) -> Result<Self, ProtoError> {
        let mut return_codes = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            if !buf.has_remaining() {
                return Err(ProtoError::BufferTooShort { need: 1, have: 0 });
            }
            return_codes.push(buf.get_u8());
        }
        Ok(WriteVarResponse { return_codes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    use crate::proto::s7::header::{Area, TransportSize};
    use crate::proto::s7::read_var::AddressItem;

    fn make_write_item(db: u16, start: u32, data: &[u8]) -> WriteItem {
        WriteItem {
            address: AddressItem {
                area: Area::DataBlock,
                db_number: db,
                start,
                bit_offset: 0,
                length: data.len() as u16,
                transport: TransportSize::Byte,
            },
            data: Bytes::copy_from_slice(data),
        }
    }

    #[test]
    fn write_var_response_decode_ok() {
        let raw: &[u8] = &[0xFF];
        let mut b = Bytes::copy_from_slice(raw);
        let resp = WriteVarResponse::decode(&mut b, 1).unwrap();
        assert_eq!(resp.return_codes[0], 0xFF);
    }

    #[test]
    fn write_var_response_decode_multi() {
        let raw: &[u8] = &[0xFF, 0xFF, 0x05]; // 0x05 = access error
        let mut b = Bytes::copy_from_slice(raw);
        let resp = WriteVarResponse::decode(&mut b, 3).unwrap();
        assert_eq!(resp.return_codes, vec![0xFF, 0xFF, 0x05]);
    }

    #[test]
    fn write_var_request_encode_structure() {
        let req = WriteVarRequest {
            items: vec![make_write_item(1, 0, &[0xDE, 0xAD])],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // func=0x05, count=1
        assert_eq!(buf[0], FUNC_WRITE_VAR);
        assert_eq!(buf[1], 1);
        // after 12-byte address item: data header at offset 14
        // reserved=0x00, transport=Byte(0x02), bit_len=16 (0x0010), data=0xDE 0xAD
        assert_eq!(buf[14], 0x00); // reserved
        assert_eq!(buf[15], TransportSize::Byte as u8);
        assert_eq!(buf[16], 0x00);
        assert_eq!(buf[17], 0x10); // 16 bits
        assert_eq!(buf[18], 0xDE);
        assert_eq!(buf[19], 0xAD);
    }

    #[test]
    fn write_var_request_odd_data_padded() {
        let req = WriteVarRequest {
            items: vec![make_write_item(1, 0, &[0xAB])], // 1 byte, odd
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // total = 2 (func+count) + 12 (addr) + 4 (data header) + 1 (data) + 1 (pad) = 20
        assert_eq!(buf.len(), 20);
        assert_eq!(buf[19], 0x00); // pad byte
    }

    #[test]
    fn write_var_response_truncated_returns_err() {
        let mut b = Bytes::new();
        assert!(WriteVarResponse::decode(&mut b, 1).is_err());
    }

    #[test]
    fn write_var_request_encode_decode_roundtrip() {
        let item = make_write_item(3, 4, &[0x11, 0x22, 0x33, 0x44]);
        let req = WriteVarRequest { items: vec![item] };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = WriteVarRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].address.db_number, 3);
        assert_eq!(decoded.items[0].address.start, 4);
        assert_eq!(decoded.items[0].data.as_ref(), &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn write_var_request_decode_wrong_func_returns_err() {
        // func=0x04 (ReadVar), not WriteVar
        let raw: &[u8] = &[0x04, 0x01];
        let mut b = Bytes::copy_from_slice(raw);
        assert!(WriteVarRequest::decode(&mut b).is_err());
    }
}
