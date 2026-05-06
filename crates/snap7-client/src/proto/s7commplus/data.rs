use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

const FIXED_LEN: usize = 14;

#[derive(Debug, Clone)]
pub struct DataArea {
    pub opcode: u8,
    pub function_code: u16,
    pub seqnum: u16,
    pub session_id: u32,
    pub transport_flags: u8,
    pub payload: Bytes,
}

impl DataArea {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(self.opcode);
        buf.put_u16(0x0000); // reserved
        buf.put_u16(self.function_code);
        buf.put_u16(0x0000); // reserved
        buf.put_u16(self.seqnum);
        buf.put_u32(self.session_id);
        buf.put_u8(self.transport_flags);
        buf.put_slice(&self.payload);
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < FIXED_LEN {
            return Err(ProtoError::BufferTooShort {
                need: FIXED_LEN,
                have: buf.len(),
            });
        }
        let opcode = buf.get_u8();
        buf.advance(2); // reserved
        let function_code = buf.get_u16();
        buf.advance(2); // reserved
        let seqnum = buf.get_u16();
        let session_id = buf.get_u32();
        let transport_flags = buf.get_u8();
        let payload = buf.copy_to_bytes(buf.remaining());
        Ok(DataArea {
            opcode,
            function_code,
            seqnum,
            session_id,
            transport_flags,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    fn sample() -> DataArea {
        DataArea {
            opcode: 0x31,
            function_code: 0x04CA,
            seqnum: 1,
            session_id: 0,
            transport_flags: 0,
            payload: Bytes::from_static(&[0xDE, 0xAD]),
        }
    }

    #[test]
    fn data_area_encode_decode_roundtrip() {
        let da = sample();
        let mut buf = BytesMut::new();
        da.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = DataArea::decode(&mut b).unwrap();
        assert_eq!(decoded.opcode, 0x31);
        assert_eq!(decoded.function_code, 0x04CA);
        assert_eq!(decoded.seqnum, 1);
        assert_eq!(decoded.session_id, 0);
        assert_eq!(decoded.transport_flags, 0);
        assert_eq!(&decoded.payload[..], &[0xDE, 0xAD]);
    }

    #[test]
    fn data_area_encode_wire_bytes() {
        let da = DataArea {
            opcode: 0x31,
            function_code: 0x04CA,
            seqnum: 0x0001,
            session_id: 0,
            transport_flags: 0,
            payload: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        da.encode(&mut buf);
        assert_eq!(
            &buf[..],
            &[0x31, 0x00, 0x00, 0x04, 0xCA, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn data_area_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(&[0x31, 0x00, 0x00]);
        assert!(DataArea::decode(&mut b).is_err());
    }
}
