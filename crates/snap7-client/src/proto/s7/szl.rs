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
    /// Encode the 8-byte UserData parameter block (Tg=grSZL, SubFun=ReadSZL).
    pub fn encode_params(&self, buf: &mut BytesMut) {
        buf.put_u8(0x00); // Head[0]
        buf.put_u8(0x01); // Head[1]
        buf.put_u8(0x12); // Head[2]
        buf.put_u8(0x04); // Plen
        buf.put_u8(0x11); // Uk
        buf.put_u8(0x44); // Tg = grSZL
        buf.put_u8(0x01); // SubFun = SFun_ReadSZL
        buf.put_u8(0x00); // Seq
    }

    /// Encode the 8-byte data section: Ret + TS + DLen + SZL-ID + SZL-Index.
    pub fn encode_data(&self, buf: &mut BytesMut) {
        buf.put_u8(0xFF);        // Ret (request marker)
        buf.put_u8(0x09);        // TS = TS_ResOctet
        buf.put_u16(0x0004);     // DLen = 4 (szl_id + szl_index)
        buf.put_u16(self.szl_id);
        buf.put_u16(self.szl_index);
    }

    /// Legacy combined encode — kept for decode symmetry in tests.
    pub fn encode(&self, buf: &mut BytesMut) {
        self.encode_params(buf);
        self.encode_data(buf);
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        // params(8) + data envelope [0xFF,0x09,0x00,0x04](4) + szl_id(2) + szl_index(2) = 16
        if buf.len() < 16 {
            return Err(ProtoError::BufferTooShort {
                need: 16,
                have: buf.len(),
            });
        }
        buf.advance(8); // skip param header
        buf.advance(4); // skip data envelope: [0xFF,0x09,0x00,0x04]
        let szl_id = buf.get_u16();
        let szl_index = buf.get_u16();
        Ok(SzlRequest { szl_id, szl_index })
    }
}

impl SzlResponse {
    /// Decode an SZL data block from a UserData SZL response.
    ///
    /// Wire format (after stripping the 4-byte data envelope):
    /// ```text
    /// [szl_id: 2] [szl_index: 2] [entry_len: 2] [entry_count: 2] [entries...]
    /// ```
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 8 {
            return Err(ProtoError::BufferTooShort {
                need: 8,
                have: buf.len(),
            });
        }
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
    fn szl_request_encode_params_length() {
        let req = SzlRequest { szl_id: 0x0011, szl_index: 0x0000 };
        let mut buf = BytesMut::new();
        req.encode_params(&mut buf);
        assert_eq!(buf.len(), 8);
    }

    #[test]
    fn szl_request_encode_data_length() {
        let req = SzlRequest { szl_id: 0x0011, szl_index: 0x0001 };
        let mut buf = BytesMut::new();
        req.encode_data(&mut buf);
        assert_eq!(buf.len(), 8);
    }

    #[test]
    fn szl_request_encode_data_contains_id_index() {
        let req = SzlRequest { szl_id: 0x001C, szl_index: 0x0000 };
        let mut buf = BytesMut::new();
        req.encode_data(&mut buf);
        let b = buf.freeze();
        assert_eq!(b[0], 0xFF); // Ret
        assert_eq!(b[1], 0x09); // TS
        assert_eq!(&b[2..4], &[0x00, 0x04]); // DLen
        assert_eq!(&b[4..6], &[0x00, 0x1C]); // szl_id BE
        assert_eq!(&b[6..8], &[0x00, 0x00]); // szl_index BE
    }

    #[test]
    fn szl_request_roundtrip() {
        let req = SzlRequest { szl_id: 0x0011, szl_index: 0x0000 };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = SzlRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.szl_id, 0x0011);
        assert_eq!(decoded.szl_index, 0x0000);
    }

    #[test]
    fn szl_response_decode_with_data() {
        // Wire format: szl_id(2) + szl_index(2) + rest becomes data
        let mut raw: Vec<u8> = vec![];
        raw.extend_from_slice(&[0x00, 0x1C]); // szl_id
        raw.extend_from_slice(&[0x00, 0x00]); // szl_index
        raw.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]); // entry data
        let mut b = Bytes::from(raw);
        let resp = SzlResponse::decode(&mut b).unwrap();
        assert_eq!(resp.szl_id, 0x001C);
        assert_eq!(resp.szl_index, 0x0000);
        assert_eq!(resp.data.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn szl_response_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x00\x1C\x00\x00\x00\x22"); // only 6 bytes, need 8
        assert!(SzlResponse::decode(&mut b).is_err());
    }

    #[test]
    fn szl_request_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x00\x01\x12");
        assert!(SzlRequest::decode(&mut b).is_err());
    }
}
