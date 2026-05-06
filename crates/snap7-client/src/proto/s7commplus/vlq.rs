use bytes::{Buf, BufMut, Bytes};

use crate::proto::ProtoError;

/// Encodes a `u32` value using Siemens VLQ (variable-length quantity).
///
/// Big-endian byte order: bit 7 of each byte is the continuation flag (1 = more bytes follow).
/// The remaining 7 bits carry payload, MSB group first.
///
/// # Examples
///
/// ```
/// use bytes::BytesMut;
/// use snap7_client::proto::s7commplus::vlq::encode_vlq;
///
/// let mut buf = BytesMut::new();
/// encode_vlq(300, &mut buf);
/// assert_eq!(&buf[..], &[0x82, 0x2C]);
/// ```
pub fn encode_vlq(value: u32, buf: &mut impl BufMut) {
    if value < 0x80 {
        buf.put_u8(value as u8);
        return;
    }
    let mut groups: [u8; 5] = [0; 5];
    let mut n = 0usize;
    let mut v = value;
    while v > 0 {
        groups[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
    }
    // groups[0] is the least-significant 7-bit group; emit most-significant first
    for i in (0..n).rev() {
        let cont = if i > 0 { 0x80 } else { 0x00 };
        buf.put_u8(groups[i] | cont);
    }
}

/// Decodes a Siemens VLQ-encoded `u32` from the front of `buf`.
///
/// Consumes only the bytes belonging to the encoded value. Returns
/// [`ProtoError::BufferTooShort`] when the buffer is exhausted mid-sequence,
/// and [`ProtoError::EncodingFailed`] when more than 5 continuation bytes are
/// encountered (overflow guard).
///
/// # Examples
///
/// ```
/// use bytes::Bytes;
/// use snap7_client::proto::s7commplus::vlq::decode_vlq;
///
/// let mut b = Bytes::from_static(&[0x82, 0x2C]);
/// assert_eq!(decode_vlq(&mut b).unwrap(), 300);
/// ```
pub fn decode_vlq(buf: &mut Bytes) -> Result<u32, ProtoError> {
    let mut result = 0u64; // use u64 accumulator to detect u32 overflow
    let mut bytes_read = 0u32;
    loop {
        if buf.is_empty() {
            return Err(ProtoError::BufferTooShort { need: 1, have: 0 });
        }
        let byte = buf.get_u8();
        result = (result << 7) | ((byte & 0x7F) as u64);
        if result > u32::MAX as u64 {
            return Err(ProtoError::EncodingFailed("VLQ value exceeds u32".into()));
        }
        if byte & 0x80 == 0 {
            return Ok(result as u32);
        }
        bytes_read += 1;
        if bytes_read >= 5 {
            return Err(ProtoError::EncodingFailed(
                "VLQ overflow: more than 5 bytes".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Bytes, BytesMut};

    #[test]
    fn vlq_single_byte() {
        let mut buf = BytesMut::new();
        encode_vlq(0x2A, &mut buf);
        assert_eq!(&buf[..], &[0x2A]);
    }

    #[test]
    fn vlq_two_bytes() {
        // 300 = 0x12C → [0x82, 0x2C]
        let mut buf = BytesMut::new();
        encode_vlq(300, &mut buf);
        assert_eq!(&buf[..], &[0x82, 0x2C]);
    }

    #[test]
    fn vlq_decode_single() {
        let mut b = Bytes::from_static(&[0x2A]);
        assert_eq!(decode_vlq(&mut b).unwrap(), 0x2A);
    }

    #[test]
    fn vlq_decode_two_bytes() {
        let mut b = Bytes::from_static(&[0x82, 0x2C]);
        assert_eq!(decode_vlq(&mut b).unwrap(), 300);
    }

    #[test]
    fn vlq_roundtrip_large() {
        for v in [
            0u32,
            1,
            127,
            128,
            255,
            300,
            16383,
            16384,
            2097151,
            2097152,
            u32::MAX,
        ] {
            let mut buf = BytesMut::new();
            encode_vlq(v, &mut buf);
            let mut b = buf.freeze();
            assert_eq!(decode_vlq(&mut b).unwrap(), v, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn vlq_decode_truncated_returns_err() {
        let mut b = Bytes::from_static(&[0x82]); // continuation bit set, no next byte
        assert!(decode_vlq(&mut b).is_err());
    }

    #[test]
    fn vlq_decode_overflow_returns_err() {
        // 5-byte sequence encoding value > u32::MAX
        let mut b = Bytes::from_static(&[0x90, 0x80, 0x80, 0x80, 0x00]);
        assert!(decode_vlq(&mut b).is_err());
    }
}
