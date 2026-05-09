use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};

/// Perform the ISO-on-TCP COTP + S7 negotiate handshake.
///
/// Active partner sends CR → waits CC → sends S7 negotiate → waits AckData.
/// Returns negotiated PDU size.
pub async fn active_handshake<T>(transport: &mut T) -> Result<u16>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // COTP CR: TPKT(4) + COTP header(7) + src-TSAP param(4) + dst-TSAP param(4) = 19 bytes
    // COTP length byte counts everything after itself: 7+4+4-1 = 14 = 0x0E
    let cr: &[u8] = &[
        0x03, 0x00, 0x00, 0x13,       // TPKT version=3, total=19
        0x0E, 0xE0,                    // COTP len=14, CR code
        0x00, 0x00,                    // dst_ref
        0x00, 0x01,                    // src_ref
        0x00,                          // class/option
        0xC1, 0x02, 0x01, 0x00,        // src TSAP: code=0xC1, len=2, data
        0xC2, 0x02, 0x01, 0x00,        // dst TSAP: code=0xC2, len=2, data
    ];
    transport.write_all(cr).await?;

    // Read TPKT header to get total length, then read rest
    let mut tpkt_hdr = [0u8; 4];
    transport.read_exact(&mut tpkt_hdr).await?;
    let total = u16::from_be_bytes([tpkt_hdr[2], tpkt_hdr[3]]) as usize;
    let mut rest = vec![0u8; total - 4];
    transport.read_exact(&mut rest).await?;
    // rest[1] should be 0xD0 (CC)
    if rest.len() < 2 || rest[1] != 0xD0 {
        return Err(Error::InvalidPdu("expected COTP CC"));
    }

    // S7 negotiate: PDU type 1 (Job), pdu_ref=1, param_len=8, data_len=0
    let negotiate: &[u8] = &[
        0x03, 0x00, 0x00, 0x19,        // TPKT total=25
        0x02, 0xF0, 0x80,              // COTP DT
        0x32, 0x01,                    // S7 magic + Job
        0x00, 0x00,                    // reserved
        0x00, 0x01,                    // pdu_ref=1
        0x00, 0x08,                    // param_len=8
        0x00, 0x00,                    // data_len=0
        0xF0, 0x00,                    // negotiate func
        0x00, 0x01,                    // max_amq_calling=1
        0x00, 0x01,                    // max_amq_called=1
        0x01, 0xE0,                    // pdu_size=480
    ];
    transport.write_all(negotiate).await?;

    // Read negotiate response
    let payload = recv_iso_frame(transport).await?;
    // payload: S7 AckData header (12 bytes) + negotiate response (8 bytes)
    if payload.len() < 20 {
        return Err(Error::InvalidPdu("negotiate response too short"));
    }
    // negotiated PDU size at offset 18-19 (header=12, skip 6 bytes of neg params)
    let pdu_size = u16::from_be_bytes([payload[18], payload[19]]);
    Ok(if pdu_size == 0 { 480 } else { pdu_size })
}

/// Perform the passive side handshake: wait for CR, send CC, handle S7 negotiate.
pub async fn passive_handshake<T>(transport: &mut T) -> Result<u16>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Read CR
    let mut tpkt_hdr = [0u8; 4];
    transport.read_exact(&mut tpkt_hdr).await?;
    let total = u16::from_be_bytes([tpkt_hdr[2], tpkt_hdr[3]]) as usize;
    let mut rest = vec![0u8; total - 4];
    transport.read_exact(&mut rest).await?;

    // Send CC (same size as CR, 19 bytes)
    let cc: &[u8] = &[
        0x03, 0x00, 0x00, 0x13,        // TPKT total=19
        0x0E, 0xD0,                    // COTP len=14, CC code
        0x00, 0x01,                    // dst_ref (echoed src_ref)
        0x00, 0x01,                    // src_ref
        0x00,
        0xC1, 0x02, 0x01, 0x00,
        0xC2, 0x02, 0x01, 0x00,
    ];
    transport.write_all(cc).await?;

    // Read S7 negotiate request
    let _neg_req = recv_iso_frame(transport).await?;

    // Send negotiate response: same structure, just AckData pdu_type
    let neg_resp: &[u8] = &[
        0x03, 0x00, 0x00, 0x1B,        // TPKT total=27
        0x02, 0xF0, 0x80,              // COTP DT
        0x32, 0x03,                    // S7 + AckData
        0x00, 0x00,                    // reserved
        0x00, 0x01,                    // pdu_ref=1
        0x00, 0x08,                    // param_len=8
        0x00, 0x00,                    // data_len=0
        0x00, 0x00,                    // error_class=0, error_code=0
        0xF0, 0x00,                    // negotiate func
        0x00, 0x01,                    // max_amq_calling=1
        0x00, 0x01,                    // max_amq_called=1
        0x01, 0xE0,                    // pdu_size=480
    ];
    transport.write_all(neg_resp).await?;

    Ok(480)
}

/// Send an S7 payload wrapped in TPKT + COTP DT.
pub async fn send_iso_frame<T: AsyncWrite + Unpin>(
    transport: &mut T,
    payload: &[u8],
) -> Result<()> {
    let total = (7 + payload.len()) as u16;
    let mut buf = BytesMut::with_capacity(7 + payload.len());
    buf.extend_from_slice(&[
        0x03, 0x00,
        (total >> 8) as u8, total as u8,
        0x02, 0xF0, 0x80,
    ]);
    buf.extend_from_slice(payload);
    transport.write_all(&buf).await?;
    Ok(())
}

/// Receive one TPKT + COTP DT frame, return the S7 payload bytes.
pub async fn recv_iso_frame<T: AsyncRead + Unpin>(transport: &mut T) -> Result<Bytes> {
    let mut header = [0u8; 4];
    transport.read_exact(&mut header).await?;
    let total = u16::from_be_bytes([header[2], header[3]]) as usize;
    if total < 7 {
        return Err(Error::InvalidPdu("frame too short"));
    }
    let mut rest = vec![0u8; total - 4];
    transport.read_exact(&mut rest).await?;
    // rest[0..3] = COTP DT (3 bytes), rest[3..] = S7 payload
    if rest.len() < 3 {
        return Err(Error::InvalidPdu("missing COTP DT"));
    }
    Ok(Bytes::from(rest[3..].to_vec()))
}
