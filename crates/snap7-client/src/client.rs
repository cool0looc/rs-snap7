use bytes::{Buf, Bytes, BytesMut};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::proto::{
    cotp::CotpPdu,
    s7::{
        clock::PlcDateTime,
        header::{Area, PduType, S7Header, TransportSize},
        read_var::{AddressItem, ReadVarRequest, ReadVarResponse},
        szl::{SzlRequest, SzlResponse},
        write_var::{WriteItem, WriteVarRequest, WriteVarResponse},
    },
    tpkt::TpktFrame,
};

use crate::{
    connection::{connect, Connection},
    error::{Error, Result},
    types::ConnectParams,
};

/// A single item in a `read_multi_vars` request.
#[derive(Debug, Clone)]
pub struct MultiReadItem {
    pub area: Area,
    pub db_number: u16,
    pub start: u32,
    pub length: u16,
    pub transport: TransportSize,
}

impl MultiReadItem {
    /// Convenience constructor for a DataBlock byte read.
    pub fn db(db: u16, start: u32, length: u16) -> Self {
        Self {
            area: Area::DataBlock,
            db_number: db,
            start,
            length,
            transport: TransportSize::Byte,
        }
    }
}

/// A single item in a `write_multi_vars` request.
#[derive(Debug, Clone)]
pub struct MultiWriteItem {
    pub area: Area,
    pub db_number: u16,
    pub start: u32,
    pub data: Bytes,
}

impl MultiWriteItem {
    /// Convenience constructor for a DataBlock byte write.
    pub fn db(db: u16, start: u32, data: impl Into<Bytes>) -> Self {
        Self {
            area: Area::DataBlock,
            db_number: db,
            start,
            data: data.into(),
        }
    }
}

struct Inner<T> {
    transport: T,
    connection: Connection,
    pdu_ref: u16,
}

pub struct S7Client<T: AsyncRead + AsyncWrite + Unpin + Send> {
    inner: Mutex<Inner<T>>,
    #[allow(dead_code)]
    params: ConnectParams,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> S7Client<T> {
    pub async fn from_transport(transport: T, params: ConnectParams) -> Result<Self> {
        let mut t = transport;
        let connection = connect(&mut t, &params).await?;
        Ok(S7Client {
            inner: Mutex::new(Inner {
                transport: t,
                connection,
                pdu_ref: 1,
            }),
            params,
        })
    }

    fn next_pdu_ref(inner: &mut Inner<T>) -> u16 {
        inner.pdu_ref = inner.pdu_ref.wrapping_add(1);
        inner.pdu_ref
    }

    async fn send_s7(
        inner: &mut Inner<T>,
        param_buf: Bytes,
        data_buf: Bytes,
        pdu_ref: u16,
        pdu_type: PduType,
    ) -> Result<()> {
        let header = S7Header {
            pdu_type,
            reserved: 0,
            pdu_ref,
            param_len: param_buf.len() as u16,
            data_len: data_buf.len() as u16,
            error_class: None,
            error_code: None,
        };
        let mut s7b = BytesMut::new();
        header.encode(&mut s7b);
        s7b.extend_from_slice(&param_buf);
        s7b.extend_from_slice(&data_buf);

        let dt = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: s7b.freeze(),
        };
        let mut cotpb = BytesMut::new();
        dt.encode(&mut cotpb);
        let tpkt = TpktFrame {
            payload: cotpb.freeze(),
        };
        let mut tb = BytesMut::new();
        tpkt.encode(&mut tb)?;
        inner.transport.write_all(&tb).await?;
        Ok(())
    }

    async fn recv_s7(inner: &mut Inner<T>) -> Result<(S7Header, Bytes)> {
        let mut tpkt_hdr = [0u8; 4];
        inner.transport.read_exact(&mut tpkt_hdr).await?;
        let total = u16::from_be_bytes([tpkt_hdr[2], tpkt_hdr[3]]) as usize;
        if total < 4 {
            return Err(Error::UnexpectedResponse);
        }
        let mut payload = vec![0u8; total - 4];
        inner.transport.read_exact(&mut payload).await?;
        let mut b = Bytes::from(payload);

        // COTP DT header: LI (1) + code (1) + tpdu_nr (1)
        if b.remaining() < 3 {
            return Err(Error::UnexpectedResponse);
        }
        let _li = b.get_u8();
        let cotp_code = b.get_u8();
        if cotp_code != 0xF0 {
            return Err(Error::UnexpectedResponse);
        }
        b.advance(1); // tpdu_nr byte

        let header = S7Header::decode(&mut b)?;
        Ok((header, b))
    }

    pub async fn db_read(&self, db: u16, start: u32, length: u16) -> Result<Bytes> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

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
        let mut param_buf = BytesMut::new();
        req.encode(&mut param_buf);

        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::Job,
        )
        .await?;

        let (header, mut body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "db_read")?;
        if body.remaining() >= 2 {
            body.advance(2); // skip param echo: func + item count
        }
        let resp = ReadVarResponse::decode(&mut body, 1)?;
        if resp.items.is_empty() {
            return Err(Error::UnexpectedResponse);
        }
        if resp.items[0].return_code != 0xFF {
            return Err(Error::PlcError {
                code: resp.items[0].return_code as u32,
                message: "item error".into(),
            });
        }
        Ok(resp.items[0].data.clone())
    }

    /// Read multiple PLC regions in one or more S7 PDU exchanges.
    ///
    /// Automatically batches items when the item count would exceed the Siemens hard
    /// limit of 20 per PDU, or when the encoded request or response would exceed the
    /// negotiated PDU size. Returns one `Bytes` per item in input order.
    ///
    /// Unlike `db_read`, this accepts any `Area` and `TransportSize`.
    pub async fn read_multi_vars(&self, items: &[MultiReadItem]) -> Result<Vec<Bytes>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        // PDU size constants (in bytes)
        // S7 header: 10, func+count: 2, per-item address: 12
        const S7_HEADER: usize = 10;
        const PARAM_OVERHEAD: usize = 2; // func + item count
        const ADDR_ITEM_SIZE: usize = 12;
        // Response data item: 4 header + data + 0/1 pad
        const DATA_ITEM_OVERHEAD: usize = 4;
        const MAX_ITEMS_PER_PDU: usize = 20;

        let mut inner = self.inner.lock().await;
        let pdu_size = inner.connection.pdu_size as usize;
        let max_req_payload = pdu_size.saturating_sub(S7_HEADER + PARAM_OVERHEAD);
        let max_resp_payload = pdu_size.saturating_sub(S7_HEADER + PARAM_OVERHEAD);

        let mut results = vec![Bytes::new(); items.len()];
        let mut batch_start = 0;

        while batch_start < items.len() {
            // Build a batch that fits within PDU limits
            let mut batch_end = batch_start;
            let mut req_bytes_used = 0usize;
            let mut resp_bytes_used = 0usize;

            while batch_end < items.len() && (batch_end - batch_start) < MAX_ITEMS_PER_PDU {
                let item = &items[batch_end];
                let item_resp_size =
                    DATA_ITEM_OVERHEAD + item.length as usize + (item.length as usize % 2);

                if batch_end > batch_start
                    && (req_bytes_used + ADDR_ITEM_SIZE > max_req_payload
                        || resp_bytes_used + item_resp_size > max_resp_payload)
                {
                    break;
                }
                req_bytes_used += ADDR_ITEM_SIZE;
                resp_bytes_used += item_resp_size;
                batch_end += 1;
            }

            let batch = &items[batch_start..batch_end];
            let pdu_ref = Self::next_pdu_ref(&mut inner);

            let req = ReadVarRequest {
                items: batch
                    .iter()
                    .map(|item| AddressItem {
                        area: item.area,
                        db_number: item.db_number,
                        start: item.start,
                        bit_offset: 0,
                        // Siemens requires Byte transport + byte-count length in the request.
                        // The item's declared transport is only used to decode the response.
                        length: item.length,
                        transport: TransportSize::Byte,
                    })
                    .collect(),
            };
            let mut param_buf = BytesMut::new();
            req.encode(&mut param_buf);

            Self::send_s7(
                &mut inner,
                param_buf.freeze(),
                Bytes::new(),
                pdu_ref,
                PduType::Job,
            )
            .await?;

            let (header, mut body) = Self::recv_s7(&mut inner).await?;
            check_plc_error(&header, "read_multi_vars")?;
            if body.remaining() >= 2 {
                body.advance(2); // skip func + item_count echo
            }
            let resp = ReadVarResponse::decode(&mut body, batch.len())?;

            for (i, item) in resp.items.into_iter().enumerate() {
                if item.return_code != 0xFF {
                    return Err(Error::PlcError {
                        code: item.return_code as u32,
                        message: format!("item {} error", batch_start + i),
                    });
                }
                results[batch_start + i] = item.data;
            }

            batch_start = batch_end;
        }

        Ok(results)
    }

    /// Write multiple PLC regions in one or more S7 PDU exchanges.
    ///
    /// Automatically batches items when the count or encoded size would exceed the
    /// negotiated PDU size or the Siemens hard limit of 20 items per PDU.
    /// Returns `Ok(())` only when all items are acknowledged with return code 0xFF.
    pub async fn write_multi_vars(&self, items: &[MultiWriteItem]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }

        const S7_HEADER: usize = 10;
        const PARAM_OVERHEAD: usize = 2; // func + item count
        const ADDR_ITEM_SIZE: usize = 12;
        const DATA_ITEM_OVERHEAD: usize = 4; // reserved + transport + bit_len (2)
        const MAX_ITEMS_PER_PDU: usize = 20;

        let mut inner = self.inner.lock().await;
        let pdu_size = inner.connection.pdu_size as usize;
        let max_payload = pdu_size.saturating_sub(S7_HEADER + PARAM_OVERHEAD);

        let mut batch_start = 0;

        while batch_start < items.len() {
            let mut batch_end = batch_start;
            let mut bytes_used = 0usize;

            while batch_end < items.len() && (batch_end - batch_start) < MAX_ITEMS_PER_PDU {
                let item = &items[batch_end];
                let data_len = item.data.len();
                let item_size = ADDR_ITEM_SIZE + DATA_ITEM_OVERHEAD + data_len + (data_len % 2);

                if batch_end > batch_start && bytes_used + item_size > max_payload {
                    break;
                }
                bytes_used += item_size;
                batch_end += 1;
            }

            let batch = &items[batch_start..batch_end];
            let pdu_ref = Self::next_pdu_ref(&mut inner);

            let req = WriteVarRequest {
                items: batch
                    .iter()
                    .map(|item| WriteItem {
                        address: AddressItem {
                            area: item.area,
                            db_number: item.db_number,
                            start: item.start,
                            bit_offset: 0,
                            length: item.data.len() as u16,
                            transport: TransportSize::Byte,
                        },
                        data: item.data.clone(),
                    })
                    .collect(),
            };
            let mut param_buf = BytesMut::new();
            req.encode(&mut param_buf);

            Self::send_s7(
                &mut inner,
                param_buf.freeze(),
                Bytes::new(),
                pdu_ref,
                PduType::Job,
            )
            .await?;

            let (header, mut body) = Self::recv_s7(&mut inner).await?;
            check_plc_error(&header, "write_multi_vars")?;
            if body.remaining() >= 2 {
                body.advance(2); // skip func + item_count echo
            }
            let resp = WriteVarResponse::decode(&mut body, batch.len())?;
            for (i, &code) in resp.return_codes.iter().enumerate() {
                if code != 0xFF {
                    return Err(Error::PlcError {
                        code: code as u32,
                        message: format!("item {} write error", batch_start + i),
                    });
                }
            }

            batch_start = batch_end;
        }

        Ok(())
    }

    pub async fn db_write(&self, db: u16, start: u32, data: &[u8]) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        let req = WriteVarRequest {
            items: vec![WriteItem {
                address: AddressItem {
                    area: Area::DataBlock,
                    db_number: db,
                    start,
                    bit_offset: 0,
                    length: data.len() as u16,
                    transport: TransportSize::Byte,
                },
                data: Bytes::copy_from_slice(data),
            }],
        };
        let mut param_buf = BytesMut::new();
        req.encode(&mut param_buf);

        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::Job,
        )
        .await?;

        let (header, mut body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "db_write")?;
        if body.has_remaining() {
            body.advance(2); // skip func + item count
        }
        let resp = WriteVarResponse::decode(&mut body, 1)?;
        if resp.return_codes[0] != 0xFF {
            return Err(Error::PlcError {
                code: resp.return_codes[0] as u32,
                message: "write error".into(),
            });
        }
        Ok(())
    }

    pub async fn read_szl(&self, szl_id: u16, szl_index: u16) -> Result<SzlResponse> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        let req = SzlRequest { szl_id, szl_index };
        let mut param_buf = BytesMut::new();
        req.encode(&mut param_buf);

        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::UserData,
        )
        .await?;

        let (_header, mut body) = Self::recv_s7(&mut inner).await?;
        if body.remaining() > 12 {
            body.advance(body.remaining() - 12);
        }
        Ok(SzlResponse::decode(&mut body)?)
    }

    pub async fn read_clock(&self) -> Result<PlcDateTime> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let mut param_buf = BytesMut::new();
        param_buf.extend_from_slice(&[0x00, 0x01, 0x12, 0x04, 0xF5, 0x00]);
        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::UserData,
        )
        .await?;
        let (_header, mut body) = Self::recv_s7(&mut inner).await?;
        if body.remaining() > 8 {
            body.advance(body.remaining() - 8);
        }
        Ok(PlcDateTime::decode(&mut body)?)
    }
}

fn check_plc_error(header: &S7Header, context: &str) -> Result<()> {
    if let (Some(ec), Some(ecd)) = (header.error_class, header.error_code) {
        if ec != 0 || ecd != 0 {
            return Err(Error::PlcError {
                code: ((ec as u32) << 8) | ecd as u32,
                message: format!("{} error", context),
            });
        }
    }
    Ok(())
}

impl S7Client<crate::transport::TcpTransport> {
    pub async fn connect(addr: SocketAddr, params: ConnectParams) -> Result<Self> {
        let transport =
            crate::transport::TcpTransport::connect(addr, params.connect_timeout).await?;
        Self::from_transport(transport, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use crate::proto::{
        cotp::CotpPdu,
        s7::{
            header::{PduType, S7Header},
            negotiate::NegotiateResponse,
        },
        tpkt::TpktFrame,
    };
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    async fn mock_plc_db_read(mut server_io: tokio::io::DuplexStream, response_data: Vec<u8>) {
        let mut buf = vec![0u8; 4096];

        // respond to COTP CR
        let _ = server_io.read(&mut buf).await;
        let cc = CotpPdu::ConnectConfirm {
            dst_ref: 1,
            src_ref: 1,
        };
        let mut cb = BytesMut::new();
        cc.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame {
            payload: cb.freeze(),
        }
        .encode(&mut tb)
        .unwrap();
        server_io.write_all(&tb).await.unwrap();

        // respond to S7 negotiate
        let _ = server_io.read(&mut buf).await;
        let neg = NegotiateResponse {
            max_amq_calling: 1,
            max_amq_called: 1,
            pdu_length: 480,
        };
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData,
            reserved: 0,
            pdu_ref: 1,
            param_len: 8,
            data_len: 0,
            error_class: Some(0),
            error_code: Some(0),
        }
        .encode(&mut s7b);
        neg.encode(&mut s7b);
        let dt = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: s7b.freeze(),
        };
        let mut cb = BytesMut::new();
        dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame {
            payload: cb.freeze(),
        }
        .encode(&mut tb)
        .unwrap();
        server_io.write_all(&tb).await.unwrap();

        // respond to db_read
        let _ = server_io.read(&mut buf).await;
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData,
            reserved: 0,
            pdu_ref: 2,
            param_len: 2,
            data_len: (4 + response_data.len()) as u16,
            error_class: Some(0),
            error_code: Some(0),
        }
        .encode(&mut s7b);
        s7b.extend_from_slice(&[0x04, 0x01]); // ReadVar func + 1 item
        s7b.put_u8(0xFF); // return_code = success
        s7b.put_u8(0x04); // transport = word
        s7b.put_u16((response_data.len() * 8) as u16);
        s7b.extend_from_slice(&response_data);
        let dt = CotpPdu::Data {
            tpdu_nr: 0,
            last: true,
            payload: s7b.freeze(),
        };
        let mut cb = BytesMut::new();
        dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame {
            payload: cb.freeze(),
        }
        .encode(&mut tb)
        .unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn db_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let expected = vec![0xDE, 0xAD, 0xBE, 0xEF];
        tokio::spawn(mock_plc_db_read(server_io, expected.clone()));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.db_read(1, 0, 4).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    /// Mock that handles COTP+Negotiate handshake then serves one multi-read response.
    async fn mock_plc_multi_read(
        mut server_io: tokio::io::DuplexStream,
        items: Vec<Vec<u8>>, // one byte vec per item
    ) {
        let mut buf = vec![0u8; 4096];

        // COTP CR
        let _ = server_io.read(&mut buf).await;
        let cc = CotpPdu::ConnectConfirm { dst_ref: 1, src_ref: 1 };
        let mut cb = BytesMut::new();
        cc.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // S7 Negotiate
        let _ = server_io.read(&mut buf).await;
        let neg = NegotiateResponse { max_amq_calling: 1, max_amq_called: 1, pdu_length: 480 };
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 1,
            param_len: 8, data_len: 0, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        neg.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // ReadMultiVar request
        let _ = server_io.read(&mut buf).await;

        // Build response data: one DataItem per input item
        let item_count = items.len() as u8;
        let mut data_bytes = BytesMut::new();
        for item_data in &items {
            data_bytes.put_u8(0xFF); // return_code OK
            data_bytes.put_u8(0x04); // transport byte
            data_bytes.put_u16((item_data.len() * 8) as u16);
            data_bytes.extend_from_slice(item_data);
            if item_data.len() % 2 != 0 {
                data_bytes.put_u8(0x00); // pad
            }
        }
        let data_len = data_bytes.len() as u16;
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len: 2, data_len, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        s7b.extend_from_slice(&[0x04, item_count]); // func + item_count
        s7b.extend_from_slice(&data_bytes);

        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn read_multi_vars_returns_all_items() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let item1 = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let item2 = vec![0x01, 0x02];
        tokio::spawn(mock_plc_multi_read(server_io, vec![item1.clone(), item2.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let items = [MultiReadItem::db(1, 0, 4), MultiReadItem::db(2, 10, 2)];
        let results = client.read_multi_vars(&items).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(&results[0][..], &item1[..]);
        assert_eq!(&results[1][..], &item2[..]);
    }

    #[tokio::test]
    async fn read_multi_vars_empty_returns_empty() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_multi_read(server_io, vec![]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let results = client.read_multi_vars(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    /// Mock that handles COTP+Negotiate then serves N write-response round-trips.
    /// `batches` is a list of item counts per round-trip; the mock sends 0xFF for each.
    async fn mock_plc_multi_write(
        mut server_io: tokio::io::DuplexStream,
        pdu_size: u16,
        batches: Vec<usize>,
    ) {
        let mut buf = vec![0u8; 65536];

        // COTP CR
        let _ = server_io.read(&mut buf).await;
        let cc = CotpPdu::ConnectConfirm { dst_ref: 1, src_ref: 1 };
        let mut cb = BytesMut::new(); cc.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // S7 Negotiate
        let _ = server_io.read(&mut buf).await;
        let neg = NegotiateResponse { max_amq_calling: 1, max_amq_called: 1, pdu_length: pdu_size };
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 1,
            param_len: 8, data_len: 0, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        neg.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // One round-trip per batch
        for (i, item_count) in batches.iter().enumerate() {
            let _ = server_io.read(&mut buf).await;
            // WriteVar response: param = func(0x05) + count; data = return_code per item
            let mut s7b = BytesMut::new();
            S7Header {
                pdu_type: PduType::AckData, reserved: 0, pdu_ref: (i + 2) as u16,
                param_len: 2, data_len: *item_count as u16,
                error_class: Some(0), error_code: Some(0),
            }.encode(&mut s7b);
            s7b.extend_from_slice(&[0x05, *item_count as u8]); // func + count
            for _ in 0..*item_count {
                s7b.put_u8(0xFF); // success
            }
            let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
            let mut cb = BytesMut::new(); dt.encode(&mut cb);
            let mut tb = BytesMut::new();
            TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
            server_io.write_all(&tb).await.unwrap();
        }
    }

    #[tokio::test]
    async fn write_multi_vars_returns_ok() {
        let (client_io, server_io) = duplex(65536);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_multi_write(server_io, 480, vec![2]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let items = [
            MultiWriteItem::db(1, 0, vec![0xAA, 0xBB, 0xCC, 0xDD]),
            MultiWriteItem::db(2, 10, vec![0x01, 0x02]),
        ];
        client.write_multi_vars(&items).await.unwrap();
    }

    #[tokio::test]
    async fn write_multi_vars_empty_returns_ok() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        // No messages exchanged after handshake — the mock just needs to satisfy connect.
        tokio::spawn(mock_plc_multi_write(server_io, 480, vec![]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.write_multi_vars(&[]).await.unwrap();
    }

    /// Items split into two round-trips when PDU budget is exhausted.
    ///
    /// PDU = 64. max_payload = 64 - 10(hdr) - 2(overhead) = 52.
    /// Each item: 12(addr) + 4(data hdr) + 20(data) = 36.
    /// Two items = 72 > 52 → must split into two 1-item batches.
    #[tokio::test]
    async fn write_multi_vars_batches_when_pdu_limit_exceeded() {
        let (client_io, server_io) = duplex(65536);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_multi_write(server_io, 64, vec![1, 1]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let items = [
            MultiWriteItem::db(1, 0, vec![0x11u8; 20]),
            MultiWriteItem::db(2, 0, vec![0x22u8; 20]),
        ];
        client.write_multi_vars(&items).await.unwrap();
    }

    /// Items are split into two round trips when response would exceed the negotiated PDU size.
    ///
    /// PDU = 64 bytes. max_resp_payload = 64 - 10(hdr) - 2(func+count) = 52 bytes.
    /// Each item with 30 bytes of data costs 4+30 = 34 bytes in the response.
    /// Two such items = 68 bytes → exceeds 52 → must split into 2 round trips.
    #[tokio::test]
    async fn read_multi_vars_batches_when_pdu_limit_exceeded() {
        use crate::proto::s7::negotiate::NegotiateResponse;

        async fn mock_split_pdu(mut server_io: tokio::io::DuplexStream) {
            let mut buf = vec![0u8; 4096];

            // COTP CR
            let _ = server_io.read(&mut buf).await;
            let cc = CotpPdu::ConnectConfirm { dst_ref: 1, src_ref: 1 };
            let mut cb = BytesMut::new(); cc.encode(&mut cb);
            let mut tb = BytesMut::new();
            TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
            server_io.write_all(&tb).await.unwrap();

            // Negotiate — PDU size 64
            let _ = server_io.read(&mut buf).await;
            let neg = NegotiateResponse {
                max_amq_calling: 1, max_amq_called: 1, pdu_length: 64,
            };
            let mut s7b = BytesMut::new();
            S7Header {
                pdu_type: PduType::AckData, reserved: 0, pdu_ref: 1,
                param_len: 8, data_len: 0, error_class: Some(0), error_code: Some(0),
            }.encode(&mut s7b);
            neg.encode(&mut s7b);
            let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
            let mut cb = BytesMut::new(); dt.encode(&mut cb);
            let mut tb = BytesMut::new();
            TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
            server_io.write_all(&tb).await.unwrap();

            // Two separate round-trips, one item each
            let payloads: &[&[u8]] = &[&[0x11u8; 30], &[0x22u8; 30]];
            for (i, payload) in payloads.iter().enumerate() {
                let _ = server_io.read(&mut buf).await;
                let bit_len = (payload.len() * 8) as u16;
                let mut data_bytes = BytesMut::new();
                data_bytes.put_u8(0xFF);
                data_bytes.put_u8(0x04);
                data_bytes.put_u16(bit_len);
                data_bytes.extend_from_slice(payload);
                if payload.len() % 2 != 0 { data_bytes.put_u8(0x00); }
                let data_len = data_bytes.len() as u16;
                let mut s7b = BytesMut::new();
                S7Header {
                    pdu_type: PduType::AckData, reserved: 0, pdu_ref: (i + 2) as u16,
                    param_len: 2, data_len, error_class: Some(0), error_code: Some(0),
                }.encode(&mut s7b);
                s7b.extend_from_slice(&[0x04, 0x01]);
                s7b.extend_from_slice(&data_bytes);
                let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
                let mut cb = BytesMut::new(); dt.encode(&mut cb);
                let mut tb = BytesMut::new();
                TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
                server_io.write_all(&tb).await.unwrap();
            }
        }

        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_split_pdu(server_io));
        let client = S7Client::from_transport(client_io, params).await.unwrap();

        let items = [MultiReadItem::db(1, 0, 30), MultiReadItem::db(2, 0, 30)];
        let results = client.read_multi_vars(&items).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(&results[0][..], &[0x11u8; 30][..]);
        assert_eq!(&results[1][..], &[0x22u8; 30][..]);
    }
}
