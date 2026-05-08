use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::proto::error::ProtoError;
use crate::proto::s7::header::{Area, TransportSize};

pub const FUNC_READ_VAR: u8 = 0x04;
pub const ITEM_SPEC: u8 = 0x12;
pub const ITEM_LEN: u8 = 0x0A;
pub const ADDR_ANY: u8 = 0x10;

#[derive(Debug, Clone)]
pub struct AddressItem {
    pub area: Area,
    pub db_number: u16,
    pub start: u32, // byte offset
    pub bit_offset: u8,
    pub length: u16, // element count
    pub transport: TransportSize,
}

#[derive(Debug, Clone)]
pub struct ReadVarRequest {
    pub items: Vec<AddressItem>,
}

#[derive(Debug, Clone)]
pub struct DataItem {
    pub return_code: u8,
    pub data: Bytes,
}

#[derive(Debug, Clone)]
pub struct ReadVarResponse {
    pub items: Vec<DataItem>,
}

impl ReadVarRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(FUNC_READ_VAR);
        buf.put_u8(self.items.len() as u8);
        for item in &self.items {
            buf.put_u8(ITEM_SPEC);
            buf.put_u8(ITEM_LEN);
            buf.put_u8(ADDR_ANY);
            buf.put_u8(item.transport as u8);
            buf.put_u16(item.length);
            buf.put_u16(item.db_number);
            buf.put_u8(item.area as u8);
            // Timer/Counter: address is element index, no bit-shift (C snap7 behavior)
            let addr_bits = match item.transport {
                TransportSize::Timer | TransportSize::Counter => item.start,
                _ => (item.start * 8) | (item.bit_offset as u32),
            };
            buf.put_u8(((addr_bits >> 16) & 0xFF) as u8);
            buf.put_u8(((addr_bits >> 8) & 0xFF) as u8);
            buf.put_u8((addr_bits & 0xFF) as u8);
        }
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 2 {
            return Err(ProtoError::BufferTooShort {
                need: 2,
                have: buf.len(),
            });
        }
        let func = buf.get_u8();
        if func != FUNC_READ_VAR {
            return Err(ProtoError::UnsupportedFunction(func));
        }
        let count = buf.get_u8() as usize;
        let mut items = Vec::with_capacity(count);
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
            buf.get_u8(); // ITEM_LEN (reserved, not validated)
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
            items.push(AddressItem {
                area,
                db_number,
                start,
                bit_offset,
                length,
                transport,
            });
        }
        Ok(ReadVarRequest { items })
    }
}

impl ReadVarResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        for item in &self.items {
            buf.put_u8(item.return_code);
            buf.put_u8(0x04); // transport size: byte
                              // bit_len: for TransportSize::Byte, bit_len = byte_count * 8.
                              // TransportSize::Bit is not supported by this encode path.
            let bit_len = (item.data.len() * 8) as u16;
            buf.put_u16(bit_len);
            buf.put_slice(&item.data);
            if !item.data.len().is_multiple_of(2) {
                buf.put_u8(0x00); // pad to even boundary
            }
        }
    }

    pub fn decode(buf: &mut Bytes, item_count: usize) -> Result<Self, ProtoError> {
        let mut items = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            if buf.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    need: 4,
                    have: buf.len(),
                });
            }
            let return_code = buf.get_u8();
            let _transport = buf.get_u8();
            let bit_len = buf.get_u16() as usize;
            let byte_len = bit_len.div_ceil(8);
            if buf.len() < byte_len {
                return Err(ProtoError::BufferTooShort {
                    need: byte_len,
                    have: buf.len(),
                });
            }
            let data = buf.split_to(byte_len);
            // S7 pads data items to even byte boundaries
            if !byte_len.is_multiple_of(2) && buf.has_remaining() {
                buf.advance(1);
            }
            items.push(DataItem { return_code, data });
        }
        Ok(ReadVarResponse { items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    use crate::proto::s7::header::{Area, TransportSize};

    fn make_item(db: u16, start: u32, len: u16) -> AddressItem {
        AddressItem {
            area: Area::DataBlock,
            db_number: db,
            start,
            bit_offset: 0,
            length: len,
            transport: TransportSize::Byte,
        }
    }

    #[test]
    fn read_var_request_roundtrip() {
        let req = ReadVarRequest {
            items: vec![make_item(1, 0, 4)],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = ReadVarRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].db_number, 1);
        assert_eq!(decoded.items[0].start, 0);
        assert_eq!(decoded.items[0].length, 4);
    }

    #[test]
    fn read_var_request_multi_item() {
        let req = ReadVarRequest {
            items: vec![make_item(1, 0, 2), make_item(2, 10, 4)],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = ReadVarRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.items.len(), 2);
        assert_eq!(decoded.items[1].db_number, 2);
        assert_eq!(decoded.items[1].start, 10);
    }

    #[test]
    fn read_var_request_byte_address_encoding() {
        // start=4, bit_offset=0 → addr_bits = 32 = 0x000020
        let req = ReadVarRequest {
            items: vec![make_item(1, 4, 1)],
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // byte 9,10,11 of the item (after func+count+spec+len+any+transport+elem+db+area) = addr bytes
        // offset in buf: 2 (func+count) + 9 = 11th byte (index 10) onwards
        assert_eq!(buf[9 + 2], 0x00); // addr high
        assert_eq!(buf[10 + 2], 0x00); // addr mid
        assert_eq!(buf[11 + 2], 0x20); // addr low = 32 = 4*8
    }

    #[test]
    fn read_var_response_decode_ok() {
        // return_code=0xFF (ok), transport=0x04 (word), bit_len=16 (2 bytes), data=0xDEAD
        let raw: &[u8] = &[0xFF, 0x04, 0x00, 0x10, 0xDE, 0xAD];
        let mut b = Bytes::copy_from_slice(raw);
        let resp = ReadVarResponse::decode(&mut b, 1).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].return_code, 0xFF);
        assert_eq!(resp.items[0].data.as_ref(), &[0xDE, 0xAD]);
    }

    #[test]
    fn read_var_response_decode_odd_length_padded() {
        // bit_len=8 (1 byte data), should consume 1 byte data + 1 pad byte
        let raw: &[u8] = &[
            0xFF, 0x02, 0x00, 0x08, 0xAB, 0x00, 0xFF, 0x02, 0x00, 0x08, 0xCD, 0x00,
        ];
        let mut b = Bytes::copy_from_slice(raw);
        let resp = ReadVarResponse::decode(&mut b, 2).unwrap();
        assert_eq!(resp.items[0].data.as_ref(), &[0xAB]);
        assert_eq!(resp.items[1].data.as_ref(), &[0xCD]);
    }

    #[test]
    fn read_var_response_encode_even_length() {
        let resp = ReadVarResponse {
            items: vec![DataItem {
                return_code: 0xFF,
                data: Bytes::copy_from_slice(&[0xDE, 0xAD]),
            }],
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        // Roundtrip: decode should give back the same data
        let mut b = buf.freeze();
        let decoded = ReadVarResponse::decode(&mut b, 1).unwrap();
        assert_eq!(decoded.items[0].return_code, 0xFF);
        assert_eq!(decoded.items[0].data.as_ref(), &[0xDE, 0xAD]);
    }

    #[test]
    fn read_var_response_encode_odd_length_padded() {
        let resp = ReadVarResponse {
            items: vec![DataItem {
                return_code: 0xFF,
                data: Bytes::copy_from_slice(&[0xAB]),
            }],
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        // return_code(1) + transport(1) + bit_len(2) + data(1) + pad(1) = 6
        assert_eq!(buf.len(), 6);
        assert_eq!(buf[5], 0x00); // pad byte
    }

    #[test]
    fn read_var_request_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x04");
        assert!(ReadVarRequest::decode(&mut b).is_err());
    }

    #[test]
    fn read_var_request_wrong_func_returns_err() {
        let raw: &[u8] = &[0x05, 0x01]; // func=WriteVar, not ReadVar
        let mut b = Bytes::copy_from_slice(raw);
        assert!(ReadVarRequest::decode(&mut b).is_err());
    }

    #[test]
    fn read_var_request_wrong_item_spec_returns_err() {
        // func=0x04, count=1, bad ITEM_SPEC=0xFF then 11 more bytes
        let mut raw = vec![0x04u8, 0x01, 0xFF];
        raw.extend_from_slice(&[
            0x0A, 0x10, 0x04, 0x00, 0x04, 0x00, 0x01, 0x84, 0x00, 0x00, 0x00,
        ]);
        let mut b = Bytes::copy_from_slice(&raw);
        assert!(ReadVarRequest::decode(&mut b).is_err());
    }
}
