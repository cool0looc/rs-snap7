use crate::proto::error::ProtoError;
use crate::proto::s7commplus::data::DataArea;
use bytes::{Bytes, BytesMut};

pub const FC_CREATE_OBJECT: u16 = 0x04CA;
pub const FC_DELETE_OBJECT: u16 = 0x04D4;
pub const FC_GET_MULTI_VAR: u16 = 0x054C;
pub const FC_SET_MULTI_VAR: u16 = 0x0542;
pub const FC_INIT_SSL: u16 = 0x05B3;

pub const OPCODE_REQUEST: u8 = 0x31;
pub const OPCODE_RESPONSE: u8 = 0x32;

#[derive(Debug)]
pub struct CreateObjectRequest {
    pub seqnum: u16,
}

impl CreateObjectRequest {
    pub fn new(seqnum: u16) -> Self {
        Self { seqnum }
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        DataArea {
            opcode: OPCODE_REQUEST,
            function_code: FC_CREATE_OBJECT,
            seqnum: self.seqnum,
            session_id: 0,
            transport_flags: 0,
            payload: Bytes::from(vec![0u8; 64]),
        }
        .encode(buf);
    }
}

#[derive(Debug)]
pub struct CreateObjectResponse {
    pub session_id: u32,
    pub raw_payload: Bytes,
}

impl CreateObjectResponse {
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        let da = DataArea::decode(buf)?;
        Ok(CreateObjectResponse {
            session_id: da.session_id,
            raw_payload: da.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn create_object_request_wire_length() {
        let req = CreateObjectRequest::new(1);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // 14-byte data area header + 64 zero bytes = 78
        assert_eq!(buf.len(), 78);
    }

    #[test]
    fn create_object_request_function_code() {
        let req = CreateObjectRequest::new(1);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // FC at bytes 3-4 (after opcode=1 + reserved=2)
        assert_eq!(u16::from_be_bytes([buf[3], buf[4]]), 0x04CA);
    }

    #[test]
    fn create_object_response_decode_from_bytes() {
        use bytes::BufMut;
        let session_id: u32 = 0x0000_001A;
        let mut buf = BytesMut::new();
        buf.put_u8(0x32); // opcode = response
        buf.put_u16(0x0000); // reserved
        buf.put_u16(0x04CA); // FC
        buf.put_u16(0x0000); // reserved
        buf.put_u16(0x0001); // seqnum
        buf.put_u32(session_id);
        buf.put_u8(0x00); // transport_flags
        let mut b = buf.freeze();
        let resp = CreateObjectResponse::decode(&mut b).unwrap();
        assert_eq!(resp.session_id, session_id);
    }
}
