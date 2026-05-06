use bytes::{Bytes, BytesMut};
use snap7_client::proto::{
    cotp::CotpPdu,
    s7::{
        header::{PduType, S7Header},
        negotiate::{NegotiateRequest, NegotiateResponse},
    },
    tpkt::TpktFrame,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};

const MAX_PDU_SIZE: u16 = 480;

// NegotiateResponse encodes as: func(1) + reserved(1) + max_amq_calling(2) + max_amq_called(2) + pdu_length(2) = 8 bytes
const NEGOTIATE_PARAM_LEN: u16 = 8;

/// Perform the server-side COTP/S7 handshake over an already-accepted transport.
///
/// Protocol steps:
///   1. Receive COTP ConnectRequest (CR)
///   2. Send COTP ConnectConfirm (CC)
///   3. Receive S7 NegotiateRequest inside a COTP Data PDU
///   4. Send S7 NegotiateResponse (AckData) with negotiated PDU size
///
/// Returns the negotiated PDU size (capped at [`MAX_PDU_SIZE`]).
pub async fn server_handshake<T>(mut transport: T) -> Result<u16>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Step 1: receive CR
    let cr = recv_tpkt_cotp(&mut transport).await?;
    let src_ref = match cr {
        CotpPdu::ConnectRequest { src_ref, .. } => src_ref,
        _ => return Err(Error::NegotiationFailed),
    };

    // Step 2: send CC — dst_ref = client's src_ref, our src_ref = 0x0001
    let cc = CotpPdu::ConnectConfirm {
        dst_ref: src_ref,
        src_ref: 0x0001,
    };
    send_tpkt_cotp(&mut transport, &cc).await?;

    // Step 3: receive S7 NegotiateRequest in a COTP Data PDU
    let mut payload = recv_cotp_data(&mut transport).await?;
    let req_header = S7Header::decode(&mut payload)?;
    if req_header.pdu_type != PduType::Job {
        return Err(Error::NegotiationFailed);
    }
    let neg_req = NegotiateRequest::decode(&mut payload)?;

    // Step 4: send S7 NegotiateResponse with negotiated (capped) PDU size
    let negotiated = neg_req.pdu_length.min(MAX_PDU_SIZE);
    let resp_header = S7Header {
        pdu_type: PduType::AckData,
        reserved: 0,
        pdu_ref: req_header.pdu_ref,
        param_len: NEGOTIATE_PARAM_LEN,
        data_len: 0,
        error_class: Some(0),
        error_code: Some(0),
    };
    let neg_resp = NegotiateResponse {
        max_amq_calling: neg_req.max_amq_calling,
        max_amq_called: neg_req.max_amq_called,
        pdu_length: negotiated,
    };
    let mut s7_buf = BytesMut::new();
    resp_header.encode(&mut s7_buf);
    neg_resp.encode(&mut s7_buf);
    send_cotp_data(&mut transport, s7_buf.freeze()).await?;

    Ok(negotiated)
}

/// Read one TPKT frame from `transport` and decode the contained COTP PDU.
pub(crate) async fn recv_tpkt_cotp<T: AsyncRead + Unpin>(transport: &mut T) -> Result<CotpPdu> {
    let mut header = [0u8; 4];
    transport.read_exact(&mut header).await?;
    if header[0] != 0x03 {
        return Err(Error::NegotiationFailed);
    }
    let total = u16::from_be_bytes([header[2], header[3]]) as usize;
    if total < 4 {
        return Err(Error::NegotiationFailed);
    }
    let payload_len = total - 4;
    let mut payload = vec![0u8; payload_len];
    transport.read_exact(&mut payload).await?;
    let mut b = Bytes::from(payload);
    CotpPdu::decode(&mut b).map_err(Error::Proto)
}

/// Read one TPKT+COTP frame and extract the Data PDU payload.
///
/// Returns an error if the COTP PDU is not a Data variant.
pub(crate) async fn recv_cotp_data<T: AsyncRead + Unpin>(transport: &mut T) -> Result<Bytes> {
    let pdu = recv_tpkt_cotp(transport).await?;
    match pdu {
        CotpPdu::Data { payload, .. } => Ok(payload),
        _ => Err(Error::NegotiationFailed),
    }
}

/// Encode `pdu` into a TPKT frame and write it to `transport`.
pub(crate) async fn send_tpkt_cotp<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu: &CotpPdu,
) -> Result<()> {
    let mut cotp_buf = BytesMut::new();
    pdu.encode(&mut cotp_buf);
    let tpkt = TpktFrame {
        payload: cotp_buf.freeze(),
    };
    let mut buf = BytesMut::new();
    tpkt.encode(&mut buf)?;
    transport.write_all(&buf).await?;
    Ok(())
}

/// Wrap `payload` in a COTP Data PDU and write it as a TPKT frame.
pub(crate) async fn send_cotp_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    payload: Bytes,
) -> Result<()> {
    let dt = CotpPdu::Data {
        tpdu_nr: 0,
        last: true,
        payload,
    };
    send_tpkt_cotp(transport, &dt).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use snap7_client::proto::{
        cotp::CotpPdu,
        s7::{
            header::{PduType, S7Header},
            negotiate::NegotiateRequest,
        },
        tpkt::TpktFrame,
    };
    use tokio::io::AsyncWriteExt;

    /// Write a COTP PDU wrapped in a TPKT frame to `writer`.
    async fn write_tpkt_cotp(writer: &mut (impl tokio::io::AsyncWrite + Unpin), cotp: &CotpPdu) {
        let mut cotp_buf = BytesMut::new();
        cotp.encode(&mut cotp_buf);
        let tpkt = TpktFrame {
            payload: cotp_buf.freeze(),
        };
        let mut buf = BytesMut::new();
        tpkt.encode(&mut buf).unwrap();
        writer.write_all(&buf).await.unwrap();
    }

    /// Write an S7 NegotiateRequest wrapped in COTP Data + TPKT to `writer`.
    async fn write_negotiate_request(
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
        pdu_length: u16,
    ) {
        let header = S7Header {
            pdu_type: PduType::Job,
            reserved: 0,
            pdu_ref: 1,
            param_len: 8,
            data_len: 0,
            error_class: None,
            error_code: None,
        };
        let req = NegotiateRequest {
            max_amq_calling: 1,
            max_amq_called: 1,
            pdu_length,
        };
        let mut s7_buf = BytesMut::new();
        header.encode(&mut s7_buf);
        req.encode(&mut s7_buf);
        let dt = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: s7_buf.freeze(),
        };
        write_tpkt_cotp(writer, &dt).await;
    }

    #[tokio::test]
    async fn handshake_completes_with_valid_client() {
        let (server_io, mut client_io) = tokio::io::duplex(4096);

        // Spawn a task that plays the role of the client.
        let client_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;

            // Send CR
            let cr = CotpPdu::ConnectRequest {
                dst_ref: 0x0000,
                src_ref: 0x0001,
                rack: 0,
                slot: 2,
            };
            write_tpkt_cotp(&mut client_io, &cr).await;

            // Read CC
            let mut hdr = [0u8; 4];
            client_io.read_exact(&mut hdr).await.unwrap();
            let total = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
            let mut body = vec![0u8; total - 4];
            client_io.read_exact(&mut body).await.unwrap();
            let mut b = Bytes::from(body);
            let cc = CotpPdu::decode(&mut b).unwrap();
            assert!(
                matches!(cc, CotpPdu::ConnectConfirm { .. }),
                "expected ConnectConfirm, got {cc:?}"
            );

            // Send NegotiateRequest
            write_negotiate_request(&mut client_io, 480).await;

            // Drain the NegotiateResponse (just read all remaining bytes)
            let mut drain = vec![0u8; 512];
            let _ = client_io.read(&mut drain).await;
        });

        let result = server_handshake(server_io).await;
        client_task.await.unwrap();
        assert!(
            result.is_ok(),
            "server_handshake returned error: {result:?}"
        );
        assert_eq!(result.unwrap(), 480);
    }

    #[tokio::test]
    async fn handshake_fails_on_non_cr() {
        let (server_io, mut client_io) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            // Send a Data PDU instead of a ConnectRequest
            let dt = CotpPdu::Data {
                tpdu_nr: 0,
                last: true,
                payload: Bytes::from_static(b"oops"),
            };
            write_tpkt_cotp(&mut client_io, &dt).await;
        });

        let result = server_handshake(server_io).await;
        assert!(result.is_err(), "expected error, got: {result:?}");
    }
}
