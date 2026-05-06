use bytes::BytesMut;
use crate::proto::{
    cotp::CotpPdu,
    s7::{
        header::{PduType, S7Header},
        negotiate::{NegotiateRequest, NegotiateResponse},
    },
    tpkt::TpktFrame,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    error::{Error, Result},
    types::ConnectParams,
};

pub struct Connection {
    pub pdu_size: u16,
}

pub async fn connect<T>(mut transport: T, params: &ConnectParams) -> Result<Connection>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    // Step 1: send COTP CR
    let cr = CotpPdu::ConnectRequest {
        dst_ref: 0x0000,
        src_ref: 0x0001,
        rack: params.rack,
        slot: params.slot,
    };
    send_cotp(&mut transport, &cr).await?;

    // Step 2: receive COTP CC
    let cc = recv_cotp(&mut transport).await?;
    if !matches!(cc, CotpPdu::ConnectConfirm { .. }) {
        return Err(Error::NegotiationFailed);
    }

    // Step 3: send S7 negotiate request
    let neg_req = NegotiateRequest {
        max_amq_calling: 1,
        max_amq_called: 1,
        pdu_length: params.pdu_size,
    };
    let mut s7_buf = BytesMut::new();
    let header = S7Header {
        pdu_type: PduType::Job,
        reserved: 0,
        pdu_ref: 1,
        param_len: 8,
        data_len: 0,
        error_class: None,
        error_code: None,
    };
    header.encode(&mut s7_buf);
    neg_req.encode(&mut s7_buf);
    send_cotp_data(&mut transport, s7_buf.freeze()).await?;

    // Step 4: receive S7 negotiate response
    let payload = recv_cotp_data(&mut transport).await?;
    let mut b = payload;
    let resp_header = S7Header::decode(&mut b)?;
    if resp_header.pdu_type != PduType::AckData {
        return Err(Error::NegotiationFailed);
    }
    if let (Some(ec), Some(ecd)) = (resp_header.error_class, resp_header.error_code) {
        if ec != 0 || ecd != 0 {
            return Err(Error::PlcError {
                code: ((ec as u32) << 8) | ecd as u32,
                message: "negotiate error".into(),
            });
        }
    }
    let neg_resp = NegotiateResponse::decode(&mut b)?;
    Ok(Connection {
        pdu_size: neg_resp.pdu_length,
    })
}

async fn send_cotp<T: AsyncWrite + Unpin>(transport: &mut T, pdu: &CotpPdu) -> Result<()> {
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

async fn send_cotp_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    payload: bytes::Bytes,
) -> Result<()> {
    let dt = CotpPdu::Data {
        tpdu_nr: 0,
        last: true,
        payload,
    };
    send_cotp(transport, &dt).await
}

async fn recv_cotp<T: AsyncRead + Unpin>(transport: &mut T) -> Result<CotpPdu> {
    let mut header = [0u8; 4];
    transport.read_exact(&mut header).await?;
    let total = u16::from_be_bytes([header[2], header[3]]) as usize;
    let payload_len = total - 4;
    let mut payload = vec![0u8; payload_len];
    transport.read_exact(&mut payload).await?;
    let mut b = bytes::Bytes::from(payload);
    Ok(CotpPdu::decode(&mut b)?)
}

async fn recv_cotp_data<T: AsyncRead + Unpin>(transport: &mut T) -> Result<bytes::Bytes> {
    let pdu = recv_cotp(transport).await?;
    match pdu {
        CotpPdu::Data { payload, .. } => Ok(payload),
        _ => Err(Error::UnexpectedResponse),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use crate::proto::{
        cotp::CotpPdu,
        s7::{
            header::{PduType, S7Header},
            negotiate::NegotiateResponse,
        },
        tpkt::TpktFrame,
    };
    use tokio::io::AsyncWriteExt;

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

    #[tokio::test]
    async fn handshake_sends_cr_receives_cc() {
        let (client_io, mut server_io) = tokio::io::duplex(4096);
        let params = crate::types::ConnectParams::default();

        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 256];
            let _ = server_io.read(&mut buf).await;
            let cc = CotpPdu::ConnectConfirm {
                dst_ref: 0x0001,
                src_ref: 0x0001,
            };
            write_tpkt_cotp(&mut server_io, &cc).await;

            let _ = server_io.read(&mut buf).await;
            let neg = NegotiateResponse {
                max_amq_calling: 1,
                max_amq_called: 1,
                pdu_length: 480,
            };
            let mut s7h = BytesMut::new();
            let header = S7Header {
                pdu_type: PduType::AckData,
                reserved: 0,
                pdu_ref: 1,
                param_len: 8,
                data_len: 0,
                error_class: Some(0),
                error_code: Some(0),
            };
            header.encode(&mut s7h);
            neg.encode(&mut s7h);
            let dt = CotpPdu::Data {
                tpdu_nr: 0,
                last: true,
                payload: s7h.freeze(),
            };
            write_tpkt_cotp(&mut server_io, &dt).await;
        });

        let result = connect(client_io, &params).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().pdu_size, 480);
    }

    #[tokio::test]
    async fn handshake_fails_when_cc_not_received() {
        let (client_io, mut server_io) = tokio::io::duplex(4096);
        let params = crate::types::ConnectParams::default();

        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 256];
            let _ = server_io.read(&mut buf).await;
            // Send ER (Error) instead of CC
            let er = CotpPdu::Error {
                dst_ref: 0,
                src_ref: 0,
                reject_cause: 0,
            };
            write_tpkt_cotp(&mut server_io, &er).await;
        });

        let result = connect(client_io, &params).await;
        assert!(result.is_err());
    }
}
