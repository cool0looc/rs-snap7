use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

pub const S7_MAGIC: u8 = 0x32;

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum PduType {
    Job = 0x01,
    Ack = 0x02,
    AckData = 0x03,
    UserData = 0x07,
}

impl TryFrom<u8> for PduType {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self, ProtoError> {
        match v {
            0x01 => Ok(PduType::Job),
            0x02 => Ok(PduType::Ack),
            0x03 => Ok(PduType::AckData),
            0x07 => Ok(PduType::UserData),
            _ => Err(ProtoError::UnsupportedPduType(v)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum Area {
    // Direct peripheral access — bypasses process image, used for force operations
    PeripheralInput = 0x80,
    ProcessInput = 0x81,
    ProcessOutput = 0x82,
    Marker = 0x83,
    DataBlock = 0x84,
    InstanceDB = 0x85,
    LocalData = 0x86,
    Counter = 0x1C,
    Timer = 0x1D,
}

impl TryFrom<u8> for Area {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self, ProtoError> {
        match v {
            0x80 => Ok(Area::PeripheralInput),
            0x81 => Ok(Area::ProcessInput),
            0x82 => Ok(Area::ProcessOutput),
            0x83 => Ok(Area::Marker),
            0x84 => Ok(Area::DataBlock),
            0x85 => Ok(Area::InstanceDB),
            0x86 => Ok(Area::LocalData),
            0x1C => Ok(Area::Counter),
            0x1D => Ok(Area::Timer),
            _ => Err(ProtoError::UnsupportedArea(v)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum TransportSize {
    Bit = 0x01,
    Byte = 0x02,
    Char = 0x03,
    Word = 0x04,
    Int = 0x05,
    DWord = 0x06,
    DInt = 0x07,
    Real = 0x08,
    Date = 0x09,
    Tod = 0x0A,
    Time = 0x0B,
    S5Time = 0x0C,
    DtL = 0x0F,
    Counter = 0x1C,
    Timer = 0x1D,
}

impl TryFrom<u8> for TransportSize {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self, ProtoError> {
        match v {
            0x01 => Ok(TransportSize::Bit),
            0x02 => Ok(TransportSize::Byte),
            0x03 => Ok(TransportSize::Char),
            0x04 => Ok(TransportSize::Word),
            0x05 => Ok(TransportSize::Int),
            0x06 => Ok(TransportSize::DWord),
            0x07 => Ok(TransportSize::DInt),
            0x08 => Ok(TransportSize::Real),
            0x09 => Ok(TransportSize::Date),
            0x0A => Ok(TransportSize::Tod),
            0x0B => Ok(TransportSize::Time),
            0x0C => Ok(TransportSize::S5Time),
            0x0F => Ok(TransportSize::DtL),
            0x1C => Ok(TransportSize::Counter),
            0x1D => Ok(TransportSize::Timer),
            _ => Err(ProtoError::UnsupportedTransportSize(v)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct S7Header {
    pub pdu_type: PduType,
    pub reserved: u16,
    pub pdu_ref: u16,
    pub param_len: u16,
    pub data_len: u16,
    pub error_class: Option<u8>,
    pub error_code: Option<u8>,
}

impl S7Header {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_u8(S7_MAGIC);
        buf.put_u8(self.pdu_type as u8);
        buf.put_u16(self.reserved);
        buf.put_u16(self.pdu_ref);
        buf.put_u16(self.param_len);
        buf.put_u16(self.data_len);
        if let (Some(ec), Some(ecd)) = (self.error_class, self.error_code) {
            buf.put_u8(ec);
            buf.put_u8(ecd);
        }
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 10 {
            return Err(ProtoError::BufferTooShort {
                need: 10,
                have: buf.len(),
            });
        }
        let magic = buf.get_u8();
        if magic != S7_MAGIC {
            return Err(ProtoError::InvalidMagic {
                expected: S7_MAGIC,
                got: magic,
            });
        }
        let pdu_type = PduType::try_from(buf.get_u8())?;
        let reserved = buf.get_u16();
        let pdu_ref = buf.get_u16();
        let param_len = buf.get_u16();
        let data_len = buf.get_u16();
        let (error_class, error_code) = match pdu_type {
            PduType::Ack | PduType::AckData if buf.remaining() >= 2 => {
                (Some(buf.get_u8()), Some(buf.get_u8()))
            }
            _ => (None, None),
        };
        Ok(S7Header {
            pdu_type,
            reserved,
            pdu_ref,
            param_len,
            data_len,
            error_class,
            error_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn s7_header_job_roundtrip() {
        let h = S7Header {
            pdu_type: PduType::Job,
            reserved: 0,
            pdu_ref: 1,
            param_len: 8,
            data_len: 0,
            error_class: None,
            error_code: None,
        };
        let mut buf = BytesMut::new();
        h.encode(&mut buf);
        assert_eq!(buf.len(), 10);
        let mut b = buf.freeze();
        let decoded = S7Header::decode(&mut b).unwrap();
        assert_eq!(decoded.pdu_type, PduType::Job);
        assert_eq!(decoded.pdu_ref, 1);
        assert_eq!(decoded.param_len, 8);
        assert!(decoded.error_class.is_none());
    }

    #[test]
    fn s7_header_ackdata_roundtrip() {
        let h = S7Header {
            pdu_type: PduType::AckData,
            reserved: 0,
            pdu_ref: 2,
            param_len: 8,
            data_len: 4,
            error_class: Some(0),
            error_code: Some(0),
        };
        let mut buf = BytesMut::new();
        h.encode(&mut buf);
        assert_eq!(buf.len(), 12);
        let mut b = buf.freeze();
        let decoded = S7Header::decode(&mut b).unwrap();
        assert_eq!(decoded.pdu_type, PduType::AckData);
        assert_eq!(decoded.error_class, Some(0));
        assert_eq!(decoded.error_code, Some(0));
    }

    #[test]
    fn s7_header_wrong_magic_returns_err() {
        let raw = &[0x00u8, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00];
        let mut b = Bytes::copy_from_slice(raw);
        assert!(S7Header::decode(&mut b).is_err());
    }

    #[test]
    fn s7_header_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x32\x01\x00\x00");
        assert!(S7Header::decode(&mut b).is_err());
    }

    #[test]
    fn pdu_type_try_from_invalid_returns_err() {
        assert!(PduType::try_from(0xFF).is_err());
    }

    #[test]
    fn area_try_from_invalid_returns_err() {
        assert!(Area::try_from(0x00).is_err());
    }

    #[test]
    fn transport_size_try_from_roundtrip() {
        assert_eq!(TransportSize::try_from(0x08).unwrap(), TransportSize::Real);
        assert!(TransportSize::try_from(0xFE).is_err());
    }
}
