use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::proto::s7commplus::frame::{S7PlusFrame, Version};
use crate::proto::s7commplus::session::{CreateObjectRequest, CreateObjectResponse};
use crate::proto::tpkt::TpktFrame;

use crate::error::Error;

/// Result of a successful S7CommPlus CreateObject handshake.
#[derive(Debug)]
pub struct PlusConnection {
    pub session_id: u32,
    pub seqnum: u16,
    pub version: Version,
}

/// Perform the S7CommPlus CreateObject handshake over `transport`.
///
/// Sends a `CreateObjectRequest` wrapped in an S7CommPlus V1 frame inside a
/// TPKT envelope, then reads the `CreateObjectResponse` and returns a
/// [`PlusConnection`] containing the negotiated `session_id`.
/// Perform the S7CommPlus CreateObject handshake and return both the negotiated
/// connection state and the transport, so the caller can store both.
pub async fn plus_connect<T>(mut transport: T) -> Result<(PlusConnection, T), Error>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // --- Build and send CreateObject request ---
    let req = CreateObjectRequest::new(1);
    let mut da_buf = BytesMut::new();
    req.encode(&mut da_buf);

    let plus_frame = S7PlusFrame {
        version: Version::V1,
        data: da_buf.freeze(),
    };
    let mut frame_buf = BytesMut::new();
    plus_frame.encode(&mut frame_buf).map_err(Error::Proto)?;

    let tpkt = TpktFrame {
        payload: frame_buf.freeze(),
    };
    let mut out = BytesMut::new();
    tpkt.encode(&mut out).map_err(Error::Proto)?;
    transport.write_all(&out).await?;

    // --- Read TPKT response: 4-byte header then payload ---
    let mut hdr = [0u8; 4];
    transport.read_exact(&mut hdr).await?;
    let total = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    let payload_len = total.saturating_sub(4);
    let mut payload = vec![0u8; payload_len];
    transport.read_exact(&mut payload).await?;

    // --- Decode S7CommPlus frame from TPKT payload ---
    let mut b = Bytes::from(payload);
    let s7plus_frame = S7PlusFrame::decode(&mut b).map_err(Error::Proto)?;

    // --- Decode CreateObject response ---
    let mut data = s7plus_frame.data.clone();
    let resp = CreateObjectResponse::decode(&mut data).map_err(Error::Proto)?;

    let conn = PlusConnection {
        session_id: resp.session_id,
        seqnum: 2, // seqnum 1 was consumed by the CreateObject request
        version: s7plus_frame.version,
    };
    Ok((conn, transport))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use tokio::io::AsyncWriteExt;

    fn build_create_object_response(session_id: u32) -> Vec<u8> {
        use bytes::BytesMut;
        use crate::proto::s7commplus::frame::{S7PlusFrame, Version};
        use crate::proto::s7commplus::session::OPCODE_RESPONSE;
        use crate::proto::tpkt::TpktFrame;

        let mut da = BytesMut::new();
        da.put_u8(OPCODE_RESPONSE); // opcode
        da.put_u16(0x0000); // reserved
        da.put_u16(0x04CA); // FC
        da.put_u16(0x0000); // reserved
        da.put_u16(0x0001); // seqnum
        da.put_u32(session_id); // session_id
        da.put_u8(0x00); // transport_flags

        let plus_frame = S7PlusFrame {
            version: Version::V1,
            data: da.freeze(),
        };
        let mut frame_buf = BytesMut::new();
        plus_frame.encode(&mut frame_buf).unwrap();

        let tpkt = TpktFrame {
            payload: frame_buf.freeze(),
        };
        let mut tpkt_buf = BytesMut::new();
        tpkt.encode(&mut tpkt_buf).unwrap();
        tpkt_buf.to_vec()
    }

    #[tokio::test]
    async fn plus_connect_extracts_session_id() {
        let expected_sid = 0xCAFEBABE_u32;
        let response = build_create_object_response(expected_sid);

        let (mut server, client_io) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut server, &mut buf).await;
            server.write_all(&response).await.unwrap();
        });

        let (conn, _transport) = plus_connect(client_io).await.unwrap();
        assert_eq!(conn.session_id, expected_sid);
    }
}
