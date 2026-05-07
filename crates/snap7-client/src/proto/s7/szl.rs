use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, Clone)]
pub struct SzlRequest {
    pub szl_id: u16,
    pub szl_index: u16,
}

#[derive(Debug, Clone)]
pub struct SzlResponse {
    pub szl_id: u16,
    pub szl_index: u16,
    pub data: Bytes,
}

impl SzlRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        // UserData parameter block for SZL read
        buf.put_u8(0x00); // param head len placeholder
        buf.put_u8(0x01); // param head
        buf.put_u8(0x12); // param length
        buf.put_u8(0x04); // type + function class
        buf.put_u8(0x11); // function number (SZL read)
        buf.put_u8(0x44); // sequence + last
        buf.put_u8(0x01); // data unit ref
        buf.put_u8(0x00); // error code
        buf.put_u16(self.szl_id);
        buf.put_u16(self.szl_index);
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 12 {
            return Err(ProtoError::BufferTooShort {
                need: 12,
                have: buf.len(),
            });
        }
        buf.advance(8); // skip param header
        let szl_id = buf.get_u16();
        let szl_index = buf.get_u16();
        Ok(SzlRequest { szl_id, szl_index })
    }
}

impl SzlResponse {
    /// Decode an SZL data block from a UserData SZL response.
    ///
    /// Wire format:
    /// ```text
    /// [block_len: 2] [szl_id: 2] [szl_index: 2] [data...]
    /// ```
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 6 {
            return Err(ProtoError::BufferTooShort {
                need: 6,
                have: buf.len(),
            });
        }
        let _block_len = buf.get_u16(); // length of remaining data in this block
        let szl_id = buf.get_u16();
        let szl_index = buf.get_u16();
        let data = buf.split_to(buf.len());
        Ok(SzlResponse {
            szl_id,
            szl_index,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn szl_request_roundtrip() {
        let req = SzlRequest {
            szl_id: 0x0011,
            szl_index: 0x0000,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = SzlRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.szl_id, 0x0011);
        assert_eq!(decoded.szl_index, 0x0000);
    }

    #[test]
    fn szl_request_encode_length() {
        let req = SzlRequest {
            szl_id: 0x0011,
            szl_index: 0x0001,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert_eq!(buf.len(), 12);
    }

    #[test]
    fn szl_response_decode_with_data() {
        // SZL data block: block_len(2) + szl_id(2) + szl_index(2) + data(4)
        // block_len = 2+2+4 = 8
        let mut raw: Vec<u8> = vec![];
        raw.extend_from_slice(&[0x00, 0x08]); // block_len = 8
        raw.extend_from_slice(&[0x00, 0x11]); // szl_id
        raw.extend_from_slice(&[0x00, 0x00]); // szl_index
        raw.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // data
        let mut b = Bytes::from(raw);
        let resp = SzlResponse::decode(&mut b).unwrap();
        assert_eq!(resp.szl_id, 0x0011);
        assert_eq!(resp.szl_index, 0x0000);
        assert_eq!(resp.data.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn szl_response_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x00\x08\x00\x11"); // only 4 bytes, need 6
        assert!(SzlResponse::decode(&mut b).is_err());
    }

    #[test]
    fn szl_request_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x00\x01\x12");
        assert!(SzlRequest::decode(&mut b).is_err());
    }
}
