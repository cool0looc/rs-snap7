use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
    pub weekday: u8, // 1=Sun .. 7=Sat
}

impl PlcDateTime {
    fn to_bcd(v: u8) -> u8 {
        ((v / 10) << 4) | (v % 10)
    }

    fn from_bcd(v: u8) -> u8 {
        ((v >> 4) * 10) + (v & 0x0F)
    }

    pub fn encode(&self, buf: &mut BytesMut) {
        let y = (self.year % 100) as u8;
        buf.put_u8(Self::to_bcd(y));
        buf.put_u8(Self::to_bcd(self.month));
        buf.put_u8(Self::to_bcd(self.day));
        buf.put_u8(Self::to_bcd(self.hour));
        buf.put_u8(Self::to_bcd(self.minute));
        buf.put_u8(Self::to_bcd(self.second));
        let ms_hi = (self.millisecond / 10) as u8;
        let ms_lo_nibble = (self.millisecond % 10) as u8;
        buf.put_u8(Self::to_bcd(ms_hi));
        buf.put_u8((ms_lo_nibble << 4) | (self.weekday & 0x0F));
    }

    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        if buf.len() < 8 {
            return Err(ProtoError::BufferTooShort {
                need: 8,
                have: buf.len(),
            });
        }
        let y_bcd = buf.get_u8();
        let y = Self::from_bcd(y_bcd) as u16;
        let year = if y >= 90 { 1900 + y } else { 2000 + y };
        let month = Self::from_bcd(buf.get_u8());
        let day = Self::from_bcd(buf.get_u8());
        let hour = Self::from_bcd(buf.get_u8());
        let minute = Self::from_bcd(buf.get_u8());
        let second = Self::from_bcd(buf.get_u8());
        let ms_byte1 = buf.get_u8();
        let ms_byte2 = buf.get_u8();
        let millisecond = (Self::from_bcd(ms_byte1) as u16) * 10 + ((ms_byte2 >> 4) as u16);
        let weekday = ms_byte2 & 0x0F;
        Ok(PlcDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
            weekday,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn clock_roundtrip_2024() {
        let dt = PlcDateTime {
            year: 2024,
            month: 6,
            day: 15,
            hour: 10,
            minute: 30,
            second: 0,
            millisecond: 0,
            weekday: 7,
        };
        let mut buf = BytesMut::new();
        dt.encode(&mut buf);
        assert_eq!(buf.len(), 8);
        let mut b = buf.freeze();
        let decoded = PlcDateTime::decode(&mut b).unwrap();
        assert_eq!(decoded, dt);
    }

    #[test]
    fn clock_roundtrip_1999() {
        // year >= 90 → 1900+y
        let dt = PlcDateTime {
            year: 1999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
            millisecond: 990,
            weekday: 6,
        };
        let mut buf = BytesMut::new();
        dt.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = PlcDateTime::decode(&mut b).unwrap();
        assert_eq!(decoded, dt);
    }

    #[test]
    fn clock_roundtrip_with_milliseconds() {
        let dt = PlcDateTime {
            year: 2024,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 123,
            weekday: 1,
        };
        let mut buf = BytesMut::new();
        dt.encode(&mut buf);
        let mut b = buf.freeze();
        let decoded = PlcDateTime::decode(&mut b).unwrap();
        assert_eq!(decoded.millisecond, 123);
    }

    #[test]
    fn clock_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(b"\x24\x06\x15");
        assert!(PlcDateTime::decode(&mut b).is_err());
    }

    #[test]
    fn clock_encode_is_8_bytes() {
        let dt = PlcDateTime {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
            weekday: 1,
        };
        let mut buf = BytesMut::new();
        dt.encode(&mut buf);
        assert_eq!(buf.len(), 8);
    }
}
