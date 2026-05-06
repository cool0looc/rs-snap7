use bytes::{Bytes, BytesMut};
use snap7_client::proto::s7::{
    header::{PduType, S7Header},
    read_var::{DataItem, ReadVarRequest, ReadVarResponse, FUNC_READ_VAR},
    write_var::{WriteVarRequest, WriteVarResponse, FUNC_WRITE_VAR},
};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    error::Result,
    handshake::{recv_cotp_data, send_cotp_data},
    store::DataStore,
};

/// Run the S7 request dispatch loop over an established transport.
///
/// Reads COTP Data PDUs, decodes S7 requests, executes them against
/// `store`, and sends S7 AckData responses. Runs until the transport
/// closes (EOF) or a fatal I/O error occurs.
pub async fn dispatch_loop<T>(mut transport: T, _pdu_size: u16, store: DataStore) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let mut payload = match recv_cotp_data(&mut transport).await {
            Ok(p) => p,
            Err(_) => return Ok(()), // EOF or transport closed — normal exit
        };

        let header = match S7Header::decode(&mut payload) {
            Ok(h) => h,
            Err(_) => {
                send_error_response(&mut transport, 0, 0x81, 0x04).await?;
                continue;
            }
        };

        // Peek at the function code (first byte of param section)
        if payload.is_empty() {
            send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
            continue;
        }
        let func = payload[0];

        match func {
            FUNC_READ_VAR => match handle_read_var(&mut payload, &store) {
                Ok((item_count, response)) => {
                    send_ack_data(
                        &mut transport,
                        header.pdu_ref,
                        FUNC_READ_VAR,
                        item_count,
                        response,
                    )
                    .await?;
                }
                Err(()) => {
                    send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
                }
            },
            FUNC_WRITE_VAR => match handle_write_var(&mut payload, &store) {
                Ok((item_count, response)) => {
                    send_ack_data(
                        &mut transport,
                        header.pdu_ref,
                        FUNC_WRITE_VAR,
                        item_count,
                        response,
                    )
                    .await?;
                }
                Err(()) => {
                    send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
                }
            },
            _ => {
                send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
            }
        }
    }
}

/// Decode a ReadVarRequest, perform reads from `store`, and return
/// `(item_count, data_bytes)` for the AckData response.
///
/// Returns `Err(())` if the request cannot be decoded; the caller must send
/// an error response rather than a success AckData.
fn handle_read_var(payload: &mut Bytes, store: &DataStore) -> std::result::Result<(u8, Bytes), ()> {
    let req = ReadVarRequest::decode(payload).map_err(|_| ())?;

    let items: Vec<DataItem> = req
        .items
        .iter()
        .map(|item| {
            let data = store.read_bytes(item.db_number, item.start, item.length as u32);
            DataItem {
                return_code: 0xFF,
                data: Bytes::from(data),
            }
        })
        .collect();

    let item_count = items.len() as u8;
    let resp = ReadVarResponse { items };
    let mut buf = BytesMut::new();
    resp.encode(&mut buf);
    Ok((item_count, buf.freeze()))
}

/// Decode a WriteVarRequest, perform writes to `store`, and return
/// `(item_count, data_bytes)` for the AckData response.
///
/// Returns `Err(())` if the request cannot be decoded; the caller must send
/// an error response rather than a success AckData.
fn handle_write_var(
    payload: &mut Bytes,
    store: &DataStore,
) -> std::result::Result<(u8, Bytes), ()> {
    let req = WriteVarRequest::decode(payload).map_err(|_| ())?;

    for item in &req.items {
        store.write_bytes(item.address.db_number, item.address.start, &item.data);
    }

    let item_count = req.items.len() as u8;
    let return_codes = vec![0xFF_u8; req.items.len()];
    let resp = WriteVarResponse { return_codes };
    // WriteVarResponse encodes as one return_code byte per item
    let mut buf = BytesMut::new();
    for &code in &resp.return_codes {
        buf.extend_from_slice(&[code]);
    }
    Ok((item_count, buf.freeze()))
}

/// Send an S7 AckData response.
///
/// The param section contains two bytes: `func` (function echo) and
/// `item_count` (number of items in the response data), matching what
/// S7 clients expect (`param_len: 2`).
async fn send_ack_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu_ref: u16,
    func: u8,
    item_count: u8,
    data: Bytes,
) -> Result<()> {
    let param: Bytes = Bytes::copy_from_slice(&[func, item_count]);
    let header = S7Header {
        pdu_type: PduType::AckData,
        reserved: 0,
        pdu_ref,
        param_len: 2,
        data_len: data.len() as u16,
        error_class: Some(0),
        error_code: Some(0),
    };
    let mut buf = BytesMut::new();
    header.encode(&mut buf);
    buf.extend_from_slice(&param);
    buf.extend_from_slice(&data);
    send_cotp_data(transport, buf.freeze()).await
}

/// Send an S7 AckData error response with empty param/data sections.
async fn send_error_response<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu_ref: u16,
    error_class: u8,
    error_code: u8,
) -> Result<()> {
    let header = S7Header {
        pdu_type: PduType::AckData,
        reserved: 0,
        pdu_ref,
        param_len: 0,
        data_len: 0,
        error_class: Some(error_class),
        error_code: Some(error_code),
    };
    let mut buf = BytesMut::new();
    header.encode(&mut buf);
    send_cotp_data(transport, buf.freeze()).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Buf, BytesMut};
    use snap7_client::proto::{
        cotp::CotpPdu,
        s7::{
            header::{Area, PduType, S7Header, TransportSize},
            read_var::{AddressItem, ReadVarRequest},
            write_var::{WriteItem, WriteVarRequest},
        },
        tpkt::TpktFrame,
    };
    use tokio::io::AsyncWriteExt;

    use crate::store::DataStore;

    /// Wrap an S7 payload in COTP Data + TPKT and write it to `writer`.
    async fn write_s7_frame(writer: &mut (impl tokio::io::AsyncWrite + Unpin), s7_payload: Bytes) {
        let dt = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: s7_payload,
        };
        let mut cotp_buf = BytesMut::new();
        dt.encode(&mut cotp_buf);
        let tpkt = TpktFrame {
            payload: cotp_buf.freeze(),
        };
        let mut buf = BytesMut::new();
        tpkt.encode(&mut buf).unwrap();
        writer.write_all(&buf).await.unwrap();
    }

    /// Read one TPKT+COTP Data frame from `reader` and return its S7 payload.
    async fn read_s7_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Bytes {
        use tokio::io::AsyncReadExt;
        let mut header = [0u8; 4];
        reader.read_exact(&mut header).await.unwrap();
        let total = u16::from_be_bytes([header[2], header[3]]) as usize;
        let mut body = vec![0u8; total - 4];
        reader.read_exact(&mut body).await.unwrap();
        let mut b = Bytes::from(body);
        let pdu = CotpPdu::decode(&mut b).unwrap();
        match pdu {
            CotpPdu::Data { payload, .. } => payload,
            _ => panic!("expected COTP Data PDU"),
        }
    }

    fn make_read_request(db: u16, start: u32, length: u16, pdu_ref: u16) -> Bytes {
        let header = S7Header {
            pdu_type: PduType::Job,
            reserved: 0,
            pdu_ref,
            param_len: 14, // 2 (func+count) + 12 (one address item)
            data_len: 0,
            error_class: None,
            error_code: None,
        };
        let req = ReadVarRequest {
            items: vec![AddressItem {
                area: Area::DataBlock,
                db_number: db,
                start,
                bit_offset: 0,
                length,
                transport: TransportSize::Byte,
            }],
        };
        let mut buf = BytesMut::new();
        header.encode(&mut buf);
        req.encode(&mut buf);
        buf.freeze()
    }

    fn make_write_request(db: u16, start: u32, data: &[u8], pdu_ref: u16) -> Bytes {
        let item = WriteItem {
            address: AddressItem {
                area: Area::DataBlock,
                db_number: db,
                start,
                bit_offset: 0,
                length: data.len() as u16,
                transport: TransportSize::Byte,
            },
            data: Bytes::copy_from_slice(data),
        };
        let req = WriteVarRequest { items: vec![item] };
        let mut param_buf = BytesMut::new();
        req.encode(&mut param_buf);
        let param_len = param_buf.len() as u16;
        let header = S7Header {
            pdu_type: PduType::Job,
            reserved: 0,
            pdu_ref,
            param_len,
            data_len: 0,
            error_class: None,
            error_code: None,
        };
        let mut buf = BytesMut::new();
        header.encode(&mut buf);
        buf.extend_from_slice(&param_buf);
        buf.freeze()
    }

    #[tokio::test]
    async fn dispatch_read_var_returns_data() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0xCA, 0xFE, 0xBA, 0xBE]);

        let (server_io, mut client_io) = tokio::io::duplex(4096);

        let store_clone = store.clone();
        let server_task =
            tokio::spawn(async move { dispatch_loop(server_io, 480, store_clone).await });

        // Send ReadVar request
        let s7_req = make_read_request(1, 0, 4, 1);
        write_s7_frame(&mut client_io, s7_req).await;

        // Read response
        let s7_resp = read_s7_frame(&mut client_io).await;
        let mut resp_bytes = s7_resp;
        let resp_header = S7Header::decode(&mut resp_bytes).unwrap();
        assert_eq!(resp_header.pdu_type, PduType::AckData);

        // Skip param section (2 bytes: func code + item count)
        resp_bytes.advance(2);

        // Parse ReadVarResponse (1 item, 4 bytes)
        let read_resp = ReadVarResponse::decode(&mut resp_bytes, 1).unwrap();
        assert_eq!(read_resp.items.len(), 1);
        assert_eq!(read_resp.items[0].data.as_ref(), &[0xCA, 0xFE, 0xBA, 0xBE]);

        // Close the client end — server will exit loop
        drop(client_io);
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn dispatch_write_var_stores_data() {
        let store = DataStore::new();

        let (server_io, mut client_io) = tokio::io::duplex(4096);

        let store_clone = store.clone();
        let server_task =
            tokio::spawn(async move { dispatch_loop(server_io, 480, store_clone).await });

        // Send WriteVar request
        let s7_req = make_write_request(2, 0, &[0x01, 0x02], 2);
        write_s7_frame(&mut client_io, s7_req).await;

        // Read AckData response
        let s7_resp = read_s7_frame(&mut client_io).await;
        let mut resp_bytes = s7_resp;
        let resp_header = S7Header::decode(&mut resp_bytes).unwrap();
        assert_eq!(resp_header.pdu_type, PduType::AckData);
        assert_eq!(resp_header.error_class, Some(0));
        assert_eq!(resp_header.error_code, Some(0));

        // Close client, wait for server
        drop(client_io);
        let _ = server_task.await;

        // Verify store has the written data
        let stored = store.read_bytes(2, 0, 2);
        assert_eq!(stored, vec![0x01, 0x02]);
    }
}
