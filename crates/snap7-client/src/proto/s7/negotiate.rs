use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

const FUNC_NEGOTIATE: u8 = 0xF0;

#[derive(Debug, Clone)]
pub struct NegotiateRequest {
    pub max_amq_calling: u16,
    pub max_amq_called: u16,
    pub pdu_length: u16,
}

impl NegotiateRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(FUNC_NEGOTIATE);
        buf.put_u8(0x00); // reserved
        buf.put_u16(self.max_amq_calling);
        buf.put_u16(self.max_amq_called);
        buf.put_u16(self.pdu_length);
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 8 {
            return Err(ProtoError::BufferTooShort {
                need: 8,
                have: buf.len(),
            });
        }
        let func = buf.get_u8();
        if func != FUNC_NEGOTIATE {
            return Err(ProtoError::UnsupportedFunction(func));
        }
        buf.get_u8(); // reserved
        let max_amq_calling = buf.get_u16();
        let max_amq_called = buf.get_u16();
        let pdu_length = buf.get_u16();
        Ok(NegotiateRequest {
            max_amq_calling,
            max_amq_called,
            pdu_length,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NegotiateResponse {
    pub max_amq_calling: u16,
    pub max_amq_called: u16,
    pub pdu_length: u16,
}

impl NegotiateResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(FUNC_NEGOTIATE);
        buf.put_u8(0x00);
        buf.put_u16(self.max_amq_calling);
        buf.put_u16(self.max_amq_called);
        buf.put_u16(self.pdu_length);
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 8 {
            return Err(ProtoError::BufferTooShort {
                need: 8,
                have: buf.len(),
            });
        }
        let func = buf.get_u8();
        if func != FUNC_NEGOTIATE {
            return Err(ProtoError::UnsupportedFunction(func));
        }
        buf.get_u8(); // reserved
        let max_amq_calling = buf.get_u16();
        let max_amq_called = buf.get_u16();
        let pdu_length = buf.get_u16();
        Ok(NegotiateResponse {
            max_amq_calling,
            max_amq_called,
            pdu_length,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn negotiate_request_roundtrip() {
        let req = NegotiateRequest {
            max_amq_calling: 1,
            max_amq_called: 1,
            pdu_length: 480,
        };
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = NegotiateRequest::decode(&mut b).unwrap();
        assert_eq!(decoded.pdu_length, 480);
        assert_eq!(decoded.max_amq_calling, 1);
    }

    #[test]
    fn negotiate_response_roundtrip() {
        let resp = NegotiateResponse {
            max_amq_calling: 1,
            max_amq_called: 1,
            pdu_length: 240,
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = NegotiateResponse::decode(&mut b).unwrap();
        assert_eq!(decoded.pdu_length, 240);
    }

    #[test]
    fn negotiate_request_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\xF0\x00\x00\x01");
        assert!(NegotiateRequest::decode(&mut b).is_err());
    }

    #[test]
    fn negotiate_request_wrong_func_returns_err() {
        let raw = &[0x04u8, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0xE0];
        let mut b = Bytes::copy_from_slice(raw);
        assert!(NegotiateRequest::decode(&mut b).is_err());
    }

    #[test]
    fn negotiate_response_wrong_func_returns_err() {
        let raw = &[0x04u8, 0x00, 0x00, 0x01, 0x00, 0x01, 0x01, 0xE0];
        let mut b = Bytes::copy_from_slice(raw);
        assert!(NegotiateResponse::decode(&mut b).is_err());
    }
}
