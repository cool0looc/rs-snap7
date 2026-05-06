use crate::proto::error::ProtoError;
use crate::proto::s7commplus::vlq::{decode_vlq, encode_vlq};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;
const DIGEST_MARKER: u8 = 0x20;
const DIGEST_LEN: usize = 32;

fn compute_digest(data: &[u8], key: &[u8]) -> [u8; DIGEST_LEN] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; DIGEST_LEN];
    out.copy_from_slice(&result);
    out
}

/// Append V1/V2 integrity tail to `buf`: [VLQ(id)][0x20][32 digest bytes].
/// The digest covers all bytes in `buf` before this call.
pub fn append_integrity_v1v2(
    buf: &mut BytesMut,
    key: &[u8],
    integrity_id: u32,
) -> Result<(), ProtoError> {
    let digest = compute_digest(&buf[..], key);
    encode_vlq(integrity_id, buf);
    buf.put_u8(DIGEST_MARKER);
    buf.put_slice(&digest);
    Ok(())
}

/// Decode integrity tail starting at current position: returns (integrity_id, digest).
pub fn decode_integrity_tail(buf: &mut Bytes) -> Result<(u32, [u8; DIGEST_LEN]), ProtoError> {
    let id = decode_vlq(buf)?;
    if buf.is_empty() || buf[0] != DIGEST_MARKER {
        return Err(ProtoError::InvalidMagic {
            expected: DIGEST_MARKER,
            got: if buf.is_empty() { 0x00 } else { buf[0] },
        });
    }
    buf.advance(1);
    if buf.len() < DIGEST_LEN {
        return Err(ProtoError::BufferTooShort {
            need: DIGEST_LEN,
            have: buf.len(),
        });
    }
    let mut digest = [0u8; DIGEST_LEN];
    digest.copy_from_slice(&buf[..DIGEST_LEN]);
    buf.advance(DIGEST_LEN);
    Ok((id, digest))
}

/// Verify the V1/V2 integrity block at the tail of `frame`.
/// Tries VLQ widths 1–5 to locate the integrity block.
/// Returns the integrity_id on success.
pub fn verify_v1v2(frame: &[u8], key: &[u8]) -> Result<u32, ProtoError> {
    for vlq_len in 1..=5usize {
        let tail_len = vlq_len + 1 + DIGEST_LEN;
        if frame.len() < tail_len {
            continue;
        }
        let data_end = frame.len() - tail_len;
        let tail = &frame[data_end..];
        if tail[vlq_len] != DIGEST_MARKER {
            continue;
        }
        let expected = compute_digest(&frame[..data_end], key);
        if tail[vlq_len + 1..] == expected {
            let mut id_bytes = Bytes::copy_from_slice(&tail[..vlq_len]);
            let id = decode_vlq(&mut id_bytes)?;
            return Ok(id);
        }
    }
    Err(ProtoError::IntegrityFailure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;

    const KEY: &[u8] = b"test_key_32bytes_0000000000000000";

    #[test]
    fn integrity_block_encode_decode_v1() {
        let data = b"payload data";
        let mut buf = BytesMut::new();
        buf.put_slice(data);
        append_integrity_v1v2(&mut buf, KEY, 1).unwrap();
        let mut b = buf.freeze();
        b.advance(data.len());
        let (id, _digest) = decode_integrity_tail(&mut b).unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn integrity_digest_is_34_bytes() {
        let mut buf = BytesMut::new();
        append_integrity_v1v2(&mut buf, KEY, 1).unwrap();
        // VLQ(1)=1 byte + 0x20=1 byte + 32 digest = 34 bytes
        assert_eq!(buf.len(), 34);
    }

    #[test]
    fn integrity_verify_v1v2_ok() {
        let data = b"test frame bytes";
        let mut buf = BytesMut::new();
        buf.put_slice(data);
        append_integrity_v1v2(&mut buf, KEY, 5).unwrap();
        let frame = buf.freeze();
        assert!(verify_v1v2(&frame, KEY).is_ok());
    }

    #[test]
    fn integrity_verify_v1v2_tampered_fails() {
        let data = b"test frame bytes";
        let mut buf = BytesMut::new();
        buf.put_slice(data);
        append_integrity_v1v2(&mut buf, KEY, 5).unwrap();
        let mut frame = buf.freeze().to_vec();
        frame[0] ^= 0xFF;
        assert!(verify_v1v2(&frame, KEY).is_err());
    }

    #[test]
    fn integrity_block_id_roundtrip() {
        for id in [0u32, 1, 127, 128, 300] {
            let mut buf = BytesMut::new();
            append_integrity_v1v2(&mut buf, KEY, id).unwrap();
            let mut b = buf.freeze();
            let (decoded_id, _) = decode_integrity_tail(&mut b).unwrap();
            assert_eq!(decoded_id, id, "id mismatch for {id}");
        }
    }
}
