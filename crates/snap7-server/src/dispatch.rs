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
                // payload[4] = method byte: 0x11 = UserData request
                if payload.len() >= 5 && payload[4] == 0x11 {
                    handle_user_data(&mut transport, header.pdu_ref, &payload, &store).await?;
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
    store: &DataStore,
) -> Result<()> {
    // payload[5] = Tg byte: low nibble = function group
    // 0x44 = grSZL, 0x47 = grClock
    let tg = if payload.len() >= 6 { payload[5] } else { 0 };
    let group = tg & 0x0F;

    match group {
        0x07 => handle_clock_user_data(transport, pdu_ref, payload, store).await,
        _ => handle_szl_user_data(transport, pdu_ref, payload).await,
    }
}

async fn handle_clock_user_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu_ref: u16,
    payload: &[u8],
    store: &DataStore,
) -> Result<()> {
    // payload[6] = subfn: 0x01 = read clock, 0x02 = set clock
    let subfn = if payload.len() >= 7 { payload[6] } else { 0 };

    if subfn == 0x02 {
        // Set clock: datetime bytes start at payload[16] (after 8-byte param + 4-byte envelope + 4 skipped)
        // Actual layout from client: param(8) + envelope[0xFF,0x09,0x00,0x08] + datetime(8)
        if payload.len() >= 20 {
            let mut dt_bytes = [0u8; 8];
            dt_bytes.copy_from_slice(&payload[12..20]);
            store.set_clock(dt_bytes);
        }
        // Respond: AckData with empty body
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
        return send_cotp_data(transport, buf.freeze()).await;
    }

    // Read clock: respond with real PLC layout:
    // pdu_type=UserData, param_len=12, data_len=4
    // datetime bytes span body[8..16] = param[8..12] + data[0..4]
    let clock = store.get_clock();
    let mut buf = BytesMut::new();
    let header = S7Header {
        pdu_type: PduType::UserData,
        reserved: 0,
        pdu_ref,
        param_len: 12,
        data_len: 4,
        error_class: None,
        error_code: None,
    };
    header.encode(&mut buf);
    // param: 8-byte echo (method=0x12=response, Tg=0x87) + first 4 datetime bytes
    buf.extend_from_slice(&[0x00, 0x01, 0x12, 0x08, 0x12, 0x87, 0x01, 0x00]);
    buf.extend_from_slice(&clock[..4]);
    // data: last 4 datetime bytes
    buf.extend_from_slice(&clock[4..]);
    send_cotp_data(transport, buf.freeze()).await
}

async fn handle_szl_user_data<T: AsyncWrite + Unpin>(
    transport: &mut T,
    pdu_ref: u16,
    payload: &[u8],
) -> Result<()> {
    // payload = param(8) + data_envelope(4) + [szl_id:2][szl_index:2]
    let szl_id = if payload.len() >= 14 {
        u16::from_be_bytes([payload[12], payload[13]])
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

fn szl_block(szl_id: u16, szl_index: u16, entry_len: u16, entries: &[u8]) -> Vec<u8> {
    let entry_count = if entry_len > 0 { (entries.len() / entry_len as usize) as u16 } else { 0 };
    let mut v = Vec::with_capacity(8 + entries.len());
    v.extend_from_slice(&szl_id.to_be_bytes());
    v.extend_from_slice(&szl_index.to_be_bytes());
    v.extend_from_slice(&entry_len.to_be_bytes());
    v.extend_from_slice(&entry_count.to_be_bytes());
    v.extend_from_slice(entries);
    v
}

fn build_szl_response(szl_id: u16) -> Vec<u8> {
    match szl_id {
        // Order code: entry_len=28 (2-byte index + 20-byte string + 6 version bytes)
        0x0011 => {
            let mut entry = vec![0u8; 28];
            entry[0] = 0x00; entry[1] = 0x01; // entry index 0x0001
            let s = b"Simulated PLC       "; // 20 chars
            entry[2..2 + s.len()].copy_from_slice(s);
            // version: v1.v2.v3 at offsets 23,24,25 (after 2-idx + 20-str + 1-pad)
            entry[23] = 1; entry[24] = 0; entry[25] = 0;
            szl_block(0x0011, 0x0000, 28, &entry)
        }
        // Protection level
        0x0032 => {
            let mut entry = vec![0u8; 16];
            // scheme_szl=3, scheme_module=3, scheme_bus=3, level=0
            entry[0] = 3; entry[2] = 3; entry[4] = 3;
            szl_block(0x0032, 0x0000, 16, &entry)
        }
        // CPU info: entry_len=34 (2-byte index + 32-byte string), 7 entries
        0x001C => {
            const SLEN: usize = 32;
            const ELEN: usize = 2 + SLEN;
            let entry_len = ELEN as u16;

            let make = |idx: u16, s: &[u8]| -> [u8; ELEN] {
                let mut e = [b' '; ELEN];
                e[0] = (idx >> 8) as u8;
                e[1] = idx as u8;
                let n = s.len().min(SLEN);
                e[2..2 + n].copy_from_slice(&s[..n]);
                e
            };

            let mut entries = Vec::with_capacity(7 * ELEN);
            entries.extend_from_slice(&make(0x0001, b"SimPLC"));           // AS name
            entries.extend_from_slice(&make(0x0002, b"CPU Simulated"));    // module type (S7-300 style)
            entries.extend_from_slice(&make(0x0003, b"SimPLC"));           // module name
            entries.extend_from_slice(&make(0x0004, b"(C) Simulated"));    // copyright
            entries.extend_from_slice(&make(0x0005, b"SIM-0000000001"));   // serial number
            entries.extend_from_slice(&make(0x0007, b"CPU Simulated"));    // canonical module type
            entries.extend_from_slice(&make(0x0008, b"SimPLC"));           // module name (dup)
            szl_block(0x001C, 0x0000, entry_len, &entries)
        }
        // CP info: index(2) + max_pdu(2) + max_conn(2) + max_mpi(4) + max_bus(4) = 14 bytes
        0x0131 => {
            let mut entry = vec![0u8; 14];
            entry[0] = 0x00; entry[1] = 0x01; // index 0x0001
            entry[2] = 0x01; entry[3] = 0xE0; // max_pdu=480
            entry[4] = 0x00; entry[5] = 0x20; // max_connections=32
            entry[6] = 0x00; entry[7] = 0x02; entry[8] = 0xDC; entry[9] = 0x6C; // max_mpi=187500
            entry[10] = 0x00; entry[11] = 0x00; entry[12] = 0x61; entry[13] = 0xA8; // max_bus=25000
            szl_block(0x0131, 0x0001, 14, &entry)
        }
        // PLC status: entries[3]=0x08 → payload[11]=0x08 (RUN), matching get_plc_status logic
        0x0424 => {
            let mut data = vec![0u8; 12];
            data[3] = 0x08; // RUN — payload[8+3]=payload[11]
            szl_block(0x0424, 0x0000, 12, &data)
        }
        _ => szl_block(szl_id, 0x0000, 0, &[]),
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
