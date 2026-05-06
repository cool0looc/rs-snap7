use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, Clone)]
pub enum CotpPdu {
    ConnectRequest {
        dst_ref: u16,
        src_ref: u16,
        rack: u8,
        slot: u8,
    },
    ConnectConfirm {
        dst_ref: u16,
        src_ref: u16,
    },
    Data {
        tpdu_nr: u8,
        last: bool,
        payload: Bytes,
    },
    Error {
        dst_ref: u16,
        src_ref: u16,
        reject_cause: u8,
    },
}

impl CotpPdu {
    const CODE_CR: u8 = 0xE0;
    const CODE_CC: u8 = 0xD0;
    const CODE_DT: u8 = 0xF0;
    const CODE_ER: u8 = 0x70;

    pub fn encode(&self, buf: &mut BytesMut) {
        match self {
            CotpPdu::ConnectRequest {
                dst_ref,
                src_ref,
                rack,
                slot,
            } => {
                let dst_tsap: u8 = ((rack & 0x07) << 5) | (slot & 0x1F);
                buf.put_u8(17); // length (not counting length byte itself)
                buf.put_u8(Self::CODE_CR);
                buf.put_u16(*dst_ref);
                buf.put_u16(*src_ref);
                buf.put_u8(0x00); // class 0, no options
                                  // src TSAP
                buf.put_u8(0xC1);
                buf.put_u8(2);
                buf.put_u8(0x01);
                buf.put_u8(0x00);
                // dst TSAP
                buf.put_u8(0xC2);
                buf.put_u8(2);
                buf.put_u8(0x01);
                buf.put_u8(dst_tsap);
                // TPDU size = 1024
                buf.put_u8(0xC0);
                buf.put_u8(1);
                buf.put_u8(0x0A);
            }
            CotpPdu::ConnectConfirm { dst_ref, src_ref } => {
                buf.put_u8(6);
                buf.put_u8(Self::CODE_CC);
                buf.put_u16(*dst_ref);
                buf.put_u16(*src_ref);
                buf.put_u8(0x00);
            }
            CotpPdu::Data {
                tpdu_nr,
                last,
                payload,
            } => {
                buf.put_u8(2); // length: code + tpdu byte
                buf.put_u8(Self::CODE_DT);
                let tpdu_byte = (tpdu_nr & 0x7F) | if *last { 0x80 } else { 0 };
                buf.put_u8(tpdu_byte);
                buf.put_slice(payload);
            }
            CotpPdu::Error {
                dst_ref,
                src_ref,
                reject_cause,
            } => {
                buf.put_u8(6);
                buf.put_u8(Self::CODE_ER);
                buf.put_u16(*dst_ref);
                buf.put_u16(*src_ref);
                buf.put_u8(*reject_cause);
            }
        }
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 2 {
            return Err(ProtoError::BufferTooShort {
                need: 2,
                have: buf.len(),
            });
        }
        let _length = buf[0] as usize;
        let code = buf[1];
        match code {
            Self::CODE_CR => {
                if buf.len() < 7 {
                    return Err(ProtoError::BufferTooShort {
                        need: 7,
                        have: buf.len(),
                    });
                }
                buf.advance(2);
                let dst_ref = buf.get_u16();
                let src_ref = buf.get_u16();
                let _class = buf.get_u8();
                let mut rack = 0u8;
                let mut slot = 0u8;
                while buf.len() >= 3 {
                    let param_code = buf.get_u8();
                    let param_len = buf.get_u8() as usize;
                    if buf.len() < param_len {
                        break;
                    }
                    let param_data = buf.split_to(param_len);
                    if param_code == 0xC2 && param_len >= 2 {
                        let tsap = param_data[1];
                        rack = (tsap >> 5) & 0x07;
                        slot = tsap & 0x1F;
                    }
                }
                Ok(CotpPdu::ConnectRequest {
                    dst_ref,
                    src_ref,
                    rack,
                    slot,
                })
            }
            Self::CODE_CC => {
                if buf.len() < 7 {
                    return Err(ProtoError::BufferTooShort {
                        need: 7,
                        have: buf.len(),
                    });
                }
                buf.advance(2);
                let dst_ref = buf.get_u16();
                let src_ref = buf.get_u16();
                buf.advance(buf.len());
                Ok(CotpPdu::ConnectConfirm { dst_ref, src_ref })
            }
            Self::CODE_DT => {
                if buf.len() < 3 {
                    return Err(ProtoError::BufferTooShort {
                        need: 3,
                        have: buf.len(),
                    });
                }
                buf.advance(2);
                let tpdu_byte = buf.get_u8();
                let tpdu_nr = tpdu_byte & 0x7F;
                let last = (tpdu_byte & 0x80) != 0;
                let payload = buf.split_to(buf.len());
                Ok(CotpPdu::Data {
                    tpdu_nr,
                    last,
                    payload,
                })
            }
            Self::CODE_ER => {
                if buf.len() < 6 {
                    return Err(ProtoError::BufferTooShort {
                        need: 6,
                        have: buf.len(),
                    });
                }
                buf.advance(2);
                let dst_ref = buf.get_u16();
                let src_ref = buf.get_u16();
                let reject_cause = if buf.has_remaining() { buf.get_u8() } else { 0 };
                Ok(CotpPdu::Error {
                    dst_ref,
                    src_ref,
                    reject_cause,
                })
            }
            _ => Err(ProtoError::UnsupportedPduType(code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn cotp_dt_roundtrip() {
        let pdu = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: Bytes::from_static(b"payload"),
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::Data {
                tpdu_nr,
                last,
                payload,
            } => {
                assert_eq!(tpdu_nr, 0);
                assert!(last);
                assert_eq!(payload.as_ref(), b"payload");
            }
            _ => panic!("expected Data"),
        }
    }

    #[test]
    fn cotp_cr_roundtrip() {
        let pdu = CotpPdu::ConnectRequest {
            dst_ref: 0x0000,
            src_ref: 0x0001,
            rack: 0,
            slot: 2,
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::ConnectRequest {
                src_ref,
                rack,
                slot,
                ..
            } => {
                assert_eq!(src_ref, 0x0001);
                assert_eq!(rack, 0);
                assert_eq!(slot, 2);
            }
            _ => panic!("expected ConnectRequest"),
        }
    }

    #[test]
    fn cotp_cc_roundtrip() {
        let pdu = CotpPdu::ConnectConfirm {
            dst_ref: 0x0001,
            src_ref: 0x0001,
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::ConnectConfirm { dst_ref, src_ref } => {
                assert_eq!(dst_ref, 0x0001);
                assert_eq!(src_ref, 0x0001);
            }
            _ => panic!("expected ConnectConfirm"),
        }
    }

    #[test]
    fn cotp_er_roundtrip() {
        let pdu = CotpPdu::Error {
            dst_ref: 0x0001,
            src_ref: 0x0001,
            reject_cause: 0x03,
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::Error { reject_cause, .. } => assert_eq!(reject_cause, 0x03),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn cotp_er_wire_format() {
        let mut buf = BytesMut::new();
        CotpPdu::Error {
            dst_ref: 0x0000,
            src_ref: 0x0001,
            reject_cause: 0x03,
        }
        .encode(&mut buf);
        // LI=6, code=0x70, dst_ref=0x0000, src_ref=0x0001, reject_cause=0x03
        assert_eq!(&buf[..], &[0x06, 0x70, 0x00, 0x00, 0x00, 0x01, 0x03]);
    }

    #[test]
    fn cotp_dt_last_false() {
        let pdu = CotpPdu::Data {
            tpdu_nr: 5,
            last: false,
            payload: Bytes::from_static(b"x"),
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::Data { tpdu_nr, last, .. } => {
                assert_eq!(tpdu_nr, 5);
                assert!(!last);
            }
            _ => panic!("expected Data"),
        }
    }

    #[test]
    fn cotp_decode_unknown_code_returns_err() {
        let raw = &[0x02u8, 0xAA, 0x00, 0x00]; // unknown code 0xAA
        let mut b = Bytes::copy_from_slice(raw);
        assert!(CotpPdu::decode(&mut b).is_err());
    }

    #[test]
    fn cotp_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x01");
        assert!(CotpPdu::decode(&mut b).is_err());
    }

    #[test]
    fn cotp_cr_rack_slot_encoding() {
        // rack=1, slot=3 → tsap = (1 << 5) | 3 = 35 = 0x23
        let pdu = CotpPdu::ConnectRequest {
            dst_ref: 0,
            src_ref: 0,
            rack: 1,
            slot: 3,
        };
        let mut buf = BytesMut::new();
        pdu.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = CotpPdu::decode(&mut b).unwrap();
        match decoded {
            CotpPdu::ConnectRequest { rack, slot, .. } => {
                assert_eq!(rack, 1);
                assert_eq!(slot, 3);
            }
            _ => panic!("expected ConnectRequest"),
        }
    }
}
