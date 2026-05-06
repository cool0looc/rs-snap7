use crate::proto::error::ProtoError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

const MAGIC: u8 = 0x72;
const HEADER_LEN: usize = 4;

/// Protocol version byte used in S7CommPlus frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
    V3,
    KeepAlive,
}

impl Version {
    pub fn as_u8(&self) -> u8 {
        match self {
            Version::V1 => 0x01,
            Version::V2 => 0x02,
            Version::V3 => 0x03,
            Version::KeepAlive => 0xFF,
        }
    }
}

impl TryFrom<u8> for Version {
    type Error = ProtoError;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0x01 => Ok(Version::V1),
            0x02 => Ok(Version::V2),
            0x03 => Ok(Version::V3),
            0xFF => Ok(Version::KeepAlive),
            other => Err(ProtoError::InvalidVersion(other)),
        }
    }
}

/// An S7CommPlus frame with a 4-byte header and matching 4-byte trailer.
///
/// Wire layout:
/// ```text
/// [0x72][version u8][data_len u16 BE] <data_bytes> [0x72][version u8][data_len u16 BE]
/// ```
#[derive(Debug, Clone)]
pub struct S7PlusFrame {
    pub version: Version,
    pub data: Bytes,
}

impl S7PlusFrame {
    /// Encode the frame (header + data + trailer) into `buf`.
    #[must_use = "encoding errors must be handled"]
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), ProtoError> {
        let data_len = self.data.len();
        if data_len > u16::MAX as usize {
            return Err(ProtoError::EncodingFailed(format!(
                "data too long: {} bytes (max {})",
                data_len,
                u16::MAX
            )));
        }
        let version_byte = self.version.as_u8();
        let len_be = data_len as u16;

        // Header
        buf.put_u8(MAGIC);
        buf.put_u8(version_byte);
        buf.put_u16(len_be);

        // Data
        buf.put_slice(&self.data);

        // Trailer (identical to header)
        buf.put_u8(MAGIC);
        buf.put_u8(version_byte);
        buf.put_u16(len_be);

        Ok(())
    }

    /// Decode a frame from the front of `buf`, consuming the bytes used.
    pub fn decode(buf: &mut Bytes) -> Result<Self, ProtoError> {
        // Need at least header (4 bytes)
        if buf.remaining() < HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                need: HEADER_LEN,
                have: buf.remaining(),
            });
        }

        // Parse header
        let h_magic = buf.get_u8();
        if h_magic != MAGIC {
            return Err(ProtoError::InvalidMagic {
                expected: MAGIC,
                got: h_magic,
            });
        }
        let h_version = buf.get_u8();
        let version = Version::try_from(h_version)?;
        let data_len = buf.get_u16() as usize;

        // Read data
        if buf.remaining() < data_len {
            return Err(ProtoError::BufferTooShort {
                need: data_len,
                have: buf.remaining(),
            });
        }
        let data = buf.copy_to_bytes(data_len);

        // Parse trailer — must be at least 4 more bytes
        if buf.remaining() < HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                need: HEADER_LEN,
                have: buf.remaining(),
            });
        }
        let t_magic = buf.get_u8();
        let t_version = buf.get_u8();
        let t_data_len = buf.get_u16() as usize;

        // Validate trailer
        if t_magic != MAGIC {
            return Err(ProtoError::InvalidMagic {
                expected: MAGIC,
                got: t_magic,
            });
        }
        if t_version != h_version || t_data_len != data_len {
            return Err(ProtoError::IntegrityFailure);
        }

        Ok(S7PlusFrame { version, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    fn make_frame(version: u8, data: &[u8]) -> S7PlusFrame {
        S7PlusFrame {
            version: Version::try_from(version).unwrap(),
            data: Bytes::copy_from_slice(data),
        }
    }

    #[test]
    fn frame_encode_v1() {
        let f = make_frame(0x01, &[0xAA, 0xBB]);
        let mut buf = BytesMut::new();
        f.encode(&mut buf).unwrap();
        assert_eq!(
            &buf[..],
            &[0x72, 0x01, 0x00, 0x02, 0xAA, 0xBB, 0x72, 0x01, 0x00, 0x02]
        );
    }

    #[test]
    fn frame_encode_keepalive() {
        let f = S7PlusFrame {
            version: Version::KeepAlive,
            data: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        f.encode(&mut buf).unwrap();
        assert_eq!(&buf[..], &[0x72, 0xFF, 0x00, 0x00, 0x72, 0xFF, 0x00, 0x00]);
    }

    #[test]
    fn frame_decode_v2_roundtrip() {
        let f = make_frame(0x02, &[0x01, 0x02, 0x03]);
        let mut buf = BytesMut::new();
        f.encode(&mut buf).unwrap();
        let mut b = buf.freeze();
        let decoded = S7PlusFrame::decode(&mut b).unwrap();
        assert_eq!(decoded.version, Version::V2);
        assert_eq!(&decoded.data[..], &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn frame_decode_wrong_magic_returns_err() {
        let mut b = Bytes::from_static(&[0x73, 0x01, 0x00, 0x00, 0x73, 0x01, 0x00, 0x00]);
        assert!(S7PlusFrame::decode(&mut b).is_err());
    }

    #[test]
    fn frame_decode_trailer_mismatch_returns_err() {
        // header version=0x01, trailer version=0x02 → error
        let mut b = Bytes::from_static(&[0x72, 0x01, 0x00, 0x00, 0x72, 0x02, 0x00, 0x00]);
        assert!(S7PlusFrame::decode(&mut b).is_err());
    }

    #[test]
    fn frame_decode_truncated_returns_err() {
        // data_len=4 but only 2 bytes available
        let mut b = Bytes::from_static(&[0x72, 0x01, 0x00, 0x04, 0xAA, 0xBB]);
        assert!(S7PlusFrame::decode(&mut b).is_err());
    }
}
