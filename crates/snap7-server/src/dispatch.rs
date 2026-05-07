use bytes::{BufMut, Bytes, BytesMut};
use snap7_client::proto::s7::{
    header::{PduType, S7Header},
    read_var::{DataItem, ReadVarRequest, ReadVarResponse, FUNC_READ_VAR},
    write_var::{WriteVarRequest, FUNC_WRITE_VAR},
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
            Err(_) => return Ok(()),
        };

        let header = match S7Header::decode(&mut payload) {
            Ok(h) => h,
            Err(_) => {
                send_error_response(&mut transport, 0, 0x81, 0x04).await?;
                continue;
            }
        };

        if payload.is_empty() {
            send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
            continue;
        }

        // Dispatch based on PDU type
        match header.pdu_type {
            PduType::Job => {
                let func = payload[0];
                match func {
                    FUNC_READ_VAR => {
                        match handle_read_var(&mut payload, &store) {
                            Ok((item_count, response)) => {
                                send_ack_data(&mut transport, header.pdu_ref, FUNC_READ_VAR, item_count, response).await?;
                            }
                            Err(()) => send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?,
                        }
                    }
                    FUNC_WRITE_VAR => {
                        match handle_write_var(&mut payload, &store) {
                            Ok((item_count, response)) => {
                                send_ack_data(&mut transport, header.pdu_ref, FUNC_WRITE_VAR, item_count, response).await?;
                            }
                            Err(()) => send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?,
                        }
                    }
                    // PLC control commands
                    0x28 | 0x29 | 0x2A | 0x31 => {
                        let hdr = S7Header {
                            pdu_type: PduType::AckData,
                            reserved: 0,
                            pdu_ref: header.pdu_ref,
                            param_len: 2,
                            data_len: if func == 0x31 { 1 } else { 0 },
                            error_class: Some(0),
                            error_code: Some(0),
                        };
                        let mut buf = BytesMut::new();
                        hdr.encode(&mut buf);
                        buf.extend_from_slice(&[func, 0x00]);
                        if func == 0x31 {
                            buf.put_u8(0x08); // status = RUN
                        }
                        send_cotp_data(&mut transport, buf.freeze()).await?;
                    }
                    // Password commands
                    0x11 | 0x12 => {
                        send_simple_ack(&mut transport, header.pdu_ref).await?;
                    }
                    _ => {
                        send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
                    }
                }
            }
            PduType::UserData => {
                // UserData: SZL, clock, block info, etc.
                if payload.len() >= 5 && (payload[4] == 0x11 || payload[4] == 0xF5) {
                    handle_user_data(&mut transport, header.pdu_ref, &payload).await?;
                } else {
                    send_simple_ack(&mut transport, header.pdu_ref).await?;
                }
            }
            _ => {
                send_error_response(&mut transport, header.pdu_ref, 0x81, 0x04).await?;
            }
        }
    }
}

// -- Read / Write handlers --------------------------------------------------

fn handle_read_var(payload: &mut Bytes, store: &DataStore) -> std::result::Result<(u8, Bytes), ()> {
    let req = ReadVarRequest::decode(payload).map_err(|_| ())?;

    let items: Vec<DataItem> = req
        .items
        .iter()
        .map(|item| {
            let area_byte = item.area as u8;
            let data = store.read_area(area_byte, item.db_number, item.start, item.length as u32);
            DataItem { return_code: 0xFF, data: Bytes::from(data) }
        })
        .collect();

    let item_count = items.len() as u8;
    let resp = ReadVarResponse { items };
    let mut buf = BytesMut::new();
    resp.encode(&mut buf);
    Ok((item_count, buf.freeze()))
}

fn handle_write_var(payload: &mut Bytes, store: &DataStore) -> std::result::Result<(u8, Bytes), ()> {
    let req = WriteVarRequest::decode(payload).map_err(|_| ())?;

    for item in &req.items {
        let area_byte = item.address.area as u8;
        store.write_area(area_byte, item.address.db_number, item.address.start, &item.data);
    }

    let item_count = req.items.len() as u8;
    let mut buf = BytesMut::new();
    for _ in 0..item_count {
        buf.put_u8(0xFF);
    }
    Ok((item_count, buf.freeze()))
}

// -- UserData : SZL responses -----------------------------------------------

async fn handle_user_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu_ref: u16,
    payload: &[u8],
) -> Result<()> {
    let szl_id = if payload.len() >= 10 {
        u16::from_be_bytes([payload[8], payload[9]])
    } else {
        0
    };

    let response_data = build_szl_response(szl_id);
    let param_len = 12u16;
    let data_len = response_data.len() as u16;

    let header = S7Header {
        pdu_type: PduType::AckData,
        reserved: 0,
        pdu_ref,
        param_len,
        data_len,
        error_class: Some(0),
        error_code: Some(0),
    };
    let mut buf = BytesMut::new();
    header.encode(&mut buf);
    if payload.len() >= 12 {
        buf.extend_from_slice(&payload[..12]);
    } else {
        buf.resize(buf.len() + param_len as usize, 0);
    }
    buf.put_u8(0xFF);
    buf.put_u8(0x04);
    buf.put_u16(data_len);
    buf.extend_from_slice(&response_data);
    send_cotp_data(transport, buf.freeze()).await
}

fn build_szl_response(szl_id: u16) -> Vec<u8> {
    match szl_id {
        0x0011 => {
            let d = vec![b' '; 20];
            let blk = (4 + d.len()) as u16;
            let mut v = Vec::with_capacity(6 + d.len());
            v.extend_from_slice(&blk.to_be_bytes());
            v.extend_from_slice(&szl_id.to_be_bytes());
            v.extend_from_slice(&[0x00, 0x00]);
            v.extend_from_slice(&d);
            v
        }
        0x0032 => {
            let pl: Vec<u8> = {
                let mut v = Vec::with_capacity(16);
                v.extend_from_slice(&[0x00; 8]); // scheme_szl + scheme_module + scheme_bus + level
                v.extend_from_slice(b"        "); // pass_word
                v
            };
            let blk = (4 + pl.len()) as u16;
            let mut v = Vec::with_capacity(6 + pl.len());
            v.extend_from_slice(&blk.to_be_bytes());
            v.extend_from_slice(&szl_id.to_be_bytes());
            v.extend_from_slice(&[0x00, 0x04]);
            v.extend_from_slice(&pl);
            v
        }
        0x001C => {
            let mut pl = vec![b' '; 122];
            let name = b"Simulated PLC";
            pl[..name.len().min(24)].copy_from_slice(&name[..name.len().min(24)]);
            let blk = (4 + pl.len()) as u16;
            let mut v = Vec::with_capacity(6 + pl.len());
            v.extend_from_slice(&blk.to_be_bytes());
            v.extend_from_slice(&szl_id.to_be_bytes());
            v.extend_from_slice(&[0x00, 0x00]);
            v.extend_from_slice(&pl);
            v
        }
        _ => {
            let pl: Vec<u8> = Vec::new();
            let blk = (4 + pl.len()) as u16;
            let mut v = Vec::with_capacity(6 + pl.len());
            v.extend_from_slice(&blk.to_be_bytes());
            v.extend_from_slice(&szl_id.to_be_bytes());
            v.extend_from_slice(&[0x00, 0x00]);
            v.extend_from_slice(&pl);
            v
        }
    }
}

// -- Helper functions -------------------------------------------------------

async fn send_simple_ack<T: AsyncWrite + Unpin>(transport: &mut T, pdu_ref: u16) -> Result<()> {
    let header = S7Header {
        pdu_type: PduType::AckData,
        reserved: 0,
        pdu_ref,
        param_len: 0,
        data_len: 0,
        error_class: Some(0),
        error_code: Some(0),
    };
    let mut buf = BytesMut::new();
    header.encode(&mut buf);
    send_cotp_data(transport, buf.freeze()).await
}

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
