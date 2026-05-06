use crate::proto::error::ProtoError;
use crate::proto::s7commplus::data::DataArea;
use crate::proto::s7commplus::session::{FC_GET_MULTI_VAR, FC_SET_MULTI_VAR, OPCODE_REQUEST};
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug)]
pub struct GetVarRequest {
    pub seqnum: u16,
    pub session_id: u32,
    pub crc: u32,
    pub lid: u32,
}

impl GetVarRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        let mut payload = BytesMut::with_capacity(9);
        payload.put_u8(0x01); // one variable
        payload.put_u32(self.crc);
        payload.put_u32(self.lid);
        DataArea {
            opcode: OPCODE_REQUEST,
            function_code: FC_GET_MULTI_VAR,
            seqnum: self.seqnum,
            session_id: self.session_id,
            transport_flags: 0,
            payload: payload.freeze(),
        }
        .encode(buf);
    }
}

#[derive(Debug)]
pub struct GetVarResponse {
    pub return_code: u8,
    pub value: Bytes,
}

impl GetVarResponse {
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        let da = DataArea::decode(buf)?;
        let mut payload = da.payload;
        if payload.is_empty() {
            return Err(ProtoError::BufferTooShort { need: 3, have: 0 });
        }
        let return_code = payload.get_u8();
        if payload.len() < 2 {
            return Err(ProtoError::BufferTooShort {
                need: 2,
                have: payload.len(),
            });
        }
        let data_len = payload.get_u16() as usize;
        if payload.len() < data_len {
            return Err(ProtoError::BufferTooShort {
                need: data_len,
                have: payload.len(),
            });
        }
        let value = payload.copy_to_bytes(data_len);
        Ok(GetVarResponse { return_code, value })
    }
}

#[derive(Debug)]
pub struct SetVarRequest {
    pub seqnum: u16,
    pub session_id: u32,
    pub crc: u32,
    pub lid: u32,
    pub value: Bytes,
}

impl SetVarRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        let mut payload = BytesMut::with_capacity(9 + 2 + self.value.len());
        payload.put_u8(0x01); // one variable
        payload.put_u32(self.crc);
        payload.put_u32(self.lid);
        payload.put_u16(self.value.len() as u16);
        payload.put_slice(&self.value);
        DataArea {
            opcode: OPCODE_REQUEST,
            function_code: FC_SET_MULTI_VAR,
            seqnum: self.seqnum,
            session_id: self.session_id,
            transport_flags: 0,
            payload: payload.freeze(),
        }
        .encode(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn get_var_request_function_code() {
        let req = GetVarRequest {
            seqnum: 1,
            session_id: 0xDEAD0001,
            crc: 0xABCD1234,
            lid: 2,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert_eq!(u16::from_be_bytes([buf[3], buf[4]]), 0x054C);
    }

    #[test]
    fn get_var_request_session_id_position() {
        let req = GetVarRequest {
            seqnum: 2,
            session_id: 0x12345678,
            crc: 0xAABBCCDD,
            lid: 1,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        // session_id at bytes 9-12 (opcode=1 + res=2 + fc=2 + res=2 + seq=2)
        let sid = u32::from_be_bytes([buf[9], buf[10], buf[11], buf[12]]);
        assert_eq!(sid, 0x12345678);
    }

    #[test]
    fn set_var_request_function_code() {
        let req = SetVarRequest {
            seqnum: 3,
            session_id: 5,
            crc: 0x11223344,
            lid: 1,
            value: Bytes::from_static(&[0x3F, 0x80, 0x00, 0x00]),
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        assert_eq!(u16::from_be_bytes([buf[3], buf[4]]), 0x0542);
    }

    #[test]
    fn get_var_response_decode() {
        use bytes::BufMut;
        let mut buf = BytesMut::new();
        buf.put_u8(0x32);
        buf.put_u16(0x0000);
        buf.put_u16(0x054C);
        buf.put_u16(0x0000);
        buf.put_u16(0x0001);
        buf.put_u32(0x00000005);
        buf.put_u8(0x00);
        buf.put_u8(0x0A); // return_code OK
        buf.put_u16(4);
        buf.put_slice(&[0x3F, 0x80, 0x00, 0x00]);
        let mut b = buf.freeze();
        let resp = GetVarResponse::decode(&mut b).unwrap();
        assert_eq!(resp.return_code, 0x0A);
        assert_eq!(&resp.value[..], &[0x3F, 0x80, 0x00, 0x00]);
    }
}
