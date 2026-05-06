use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug)]
pub struct TpktFrame {
    pub payload: Bytes,
}

impl TpktFrame {
    const VERSION: u8 = 0x03;
    const HEADER_LEN: usize = 4;

    #[must_use = "encoding errors must be handled"]
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), ProtoError> {
        let total = Self::HEADER_LEN + self.payload.len();
        if total > u16::MAX as usize {
            return Err(ProtoError::EncodingFailed(format!(
                "frame too large: {total} bytes exceeds TPKT maximum of 65535"
            )));
        }
        buf.put_u8(Self::VERSION);
        buf.put_u8(0x00); // reserved
        buf.put_u16(total as u16);
        buf.put_slice(&self.payload);
        Ok(())
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < Self::HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                need: Self::HEADER_LEN,
                have: buf.len(),
            });
        }
        let version = buf[0];
        if version != Self::VERSION {
            return Err(ProtoError::InvalidMagic {
                expected: Self::VERSION,
                got: version,
            });
        }
        let total = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        if total < Self::HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                need: Self::HEADER_LEN,
                have: total,
            });
        }
        if buf.len() < total {
            return Err(ProtoError::BufferTooShort {
                need: total,
                have: buf.len(),
            });
        }
        buf.advance(Self::HEADER_LEN);
        let payload = buf.split_to(total - Self::HEADER_LEN);
        Ok(TpktFrame { payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpkt_roundtrip() {
        let frame = TpktFrame {
            payload: Bytes::from_static(b"hello"),
        };
        let mut buf = BytesMut::new();
        frame.encode(&mut buf).unwrap();
        let mut b = buf.freeze();
        let decoded = TpktFrame::decode(&mut b).unwrap();
        assert_eq!(decoded.payload.as_ref(), b"hello");
    }

    #[test]
    fn tpkt_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x03\x00");
        assert!(TpktFrame::decode(&mut b).is_err());
    }

    #[test]
    fn tpkt_decode_wrong_version_returns_err() {
        let mut b = Bytes::from_static(b"\x02\x00\x00\x05hi");
        assert!(TpktFrame::decode(&mut b).is_err());
    }

    #[test]
    fn tpkt_encode_empty_payload() {
        let frame = TpktFrame {
            payload: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        frame.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), 4);
        assert_eq!(&buf[..], &[0x03, 0x00, 0x00, 0x04]);
    }

    #[test]
    fn tpkt_decode_length_too_small_returns_err() {
        // total=2, which is less than header length of 4
        let mut b = Bytes::from_static(b"\x03\x00\x00\x02");
        assert!(TpktFrame::decode(&mut b).is_err());
    }

    #[test]
    fn tpkt_encode_oversized_returns_err() {
        let big_payload = Bytes::from(vec![0u8; 65532]); // 65532 + 4 = 65536 > u16::MAX
        let frame = TpktFrame {
            payload: big_payload,
        };
        let mut buf = BytesMut::new();
        assert!(frame.encode(&mut buf).is_err());
    }
}
