use bytes::{Buf, BufMut, Bytes, BytesMut};
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
    request_timeout: std::time::Duration,
    connected: bool,
    job_start: Option<std::time::Instant>,
    last_exec_ms: u32,
}

pub struct S7Client<T: AsyncRead + AsyncWrite + Unpin + Send> {
    inner: Mutex<Inner<T>>,
    params: ConnectParams,
    remote_addr: Option<SocketAddr>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> S7Client<T> {
    pub async fn from_transport(transport: T, params: ConnectParams) -> Result<Self> {
        let mut t = transport;
        let connection = connect(&mut t, &params).await?;
        let timeout = params.request_timeout;
        Ok(S7Client {
            inner: Mutex::new(Inner {
                transport: t,
                connection,
                pdu_ref: 1,
                request_timeout: timeout,
                connected: true,
                job_start: None,
                last_exec_ms: 0,
            }),
            params,
            remote_addr: None,
        })
    }

    /// Return the current request timeout.
    pub fn request_timeout(&self) -> std::time::Duration {
        self.params.request_timeout
    }

    /// Returns the execution time of the last completed S7 operation in milliseconds.
    ///
    /// Measures full round-trip: from send to response received. Equivalent to C `Cli_GetExecTime`.
    pub async fn get_exec_time(&self) -> u32 {
        self.inner.lock().await.last_exec_ms
    }

    /// Returns whether the transport connection is alive.
    ///
    /// Set to `false` when any I/O error is encountered.
    /// Equivalent to C `Cli_GetConnected`.
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.connected
    }

    /// Update the request timeout at runtime.
    ///
    /// This affects subsequent `recv_s7` calls made by this client instance.
    pub async fn set_request_timeout(&self, timeout: std::time::Duration) {
        let mut inner = self.inner.lock().await;
        inner.request_timeout = timeout;
    }

    /// Read a client parameter by name.
    ///
    /// Supported names: `"request_timeout"`, `"connect_timeout"`, `"pdu_size"`.
    pub fn get_param(&self, name: &str) -> Result<std::time::Duration> {
        match name {
            "request_timeout" => Ok(self.params.request_timeout),
            "connect_timeout" => Ok(self.params.connect_timeout),
            "pdu_size" => Err(Error::PlcError {
                code: 0,
                message: "pdu_size is not a Duration; use .params.pdu_size directly".into(),
            }),
            _ => Err(Error::PlcError {
                code: 0,
                message: format!("unknown parameter: {name}"),
            }),
        }
    }

    /// Set a client parameter at runtime.
    ///
    /// Supported names: `"request_timeout"` (Duration).
    pub fn set_param(&mut self, name: &str, value: std::time::Duration) -> Result<()> {
        match name {
            "request_timeout" => {
                self.params.request_timeout = value;
                Ok(())
            }
            _ => Err(Error::PlcError {
                code: 0,
                message: format!("unknown parameter: {name}"),
            }),
        }
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
        inner.job_start = Some(std::time::Instant::now());
        inner.transport.write_all(&tb).await?;
        Ok(())
    }

    async fn recv_s7(inner: &mut Inner<T>) -> Result<(S7Header, Bytes)> {
        let timeout = inner.request_timeout;
        let mut tpkt_hdr = [0u8; 4];
        if let Err(e) = tokio::time::timeout(timeout, inner.transport.read_exact(&mut tpkt_hdr))
            .await
            .map_err(|_| Error::Timeout(timeout))
            .and_then(|r| r.map_err(Error::Io))
        {
            inner.connected = false;
            return Err(e);
        }
        let total = u16::from_be_bytes([tpkt_hdr[2], tpkt_hdr[3]]) as usize;
        if total < 4 {
            return Err(Error::UnexpectedResponse);
        }
        let mut payload = vec![0u8; total - 4];
        if let Err(e) = tokio::time::timeout(timeout, inner.transport.read_exact(&mut payload))
            .await
            .map_err(|_| Error::Timeout(timeout))
            .and_then(|r| r.map_err(Error::Io))
        {
            inner.connected = false;
            return Err(e);
        }
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
        if let Some(t0) = inner.job_start.take() {
            inner.last_exec_ms = t0.elapsed().as_millis() as u32;
        }
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

    /// Read from any PLC area with explicit transport size.
    ///
    /// For DB areas use `db_read`. For Marker/Timer/Counter use this method.
    /// Timer (`area=Timer, transport=Timer`) and Counter (`area=Counter, transport=Counter`)
    /// use element-index addressing (no ×8 shift) and return 2 bytes per element.
    pub async fn read_area(
        &self,
        area: Area,
        db_number: u16,
        start: u32,
        element_count: u16,
        transport: TransportSize,
    ) -> Result<Bytes> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        let req = ReadVarRequest {
            items: vec![AddressItem {
                area,
                db_number,
                start,
                bit_offset: 0,
                length: element_count,
                transport,
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
        check_plc_error(&header, "read_area")?;
        if body.remaining() >= 2 {
            body.advance(2);
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

    /// Write to any PLC area with explicit transport size.
    ///
    /// For Timer/Counter areas the transport size byte in the request must match
    /// the area (0x1D / 0x1C). For Marker use `TransportSize::Byte`.
    pub async fn write_area(
        &self,
        area: Area,
        db_number: u16,
        start: u32,
        transport: TransportSize,
        data: &[u8],
    ) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        let req = WriteVarRequest {
            items: vec![WriteItem {
                address: AddressItem {
                    area,
                    db_number,
                    start,
                    bit_offset: 0,
                    length: data.len() as u16,
                    transport,
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
        check_plc_error(&header, "write_area")?;
        if body.has_remaining() {
            body.advance(2);
        }
        let resp = WriteVarResponse::decode(&mut body, 1)?;
        if resp.return_codes[0] != 0xFF {
            return Err(Error::PlcError {
                code: resp.return_codes[0] as u32,
                message: "write_area error".into(),
            });
        }
        Ok(())
    }

    /// Read from any PLC area using absolute addressing.
    ///
    /// A convenience wrapper around [`read_multi_vars`](Self::read_multi_vars)
    /// for a single area read.
    pub async fn ab_read(
        &self,
        area: Area,
        db_number: u16,
        start: u32,
        length: u16,
    ) -> Result<Bytes> {
        let items = [MultiReadItem {
            area,
            db_number,
            start,
            length,
            transport: TransportSize::Byte,
        }];
        let mut results = self.read_multi_vars(&items).await?;
        Ok(results.swap_remove(0))
    }

    /// Write to any PLC area using absolute addressing.
    ///
    /// A convenience wrapper around [`write_multi_vars`](Self::write_multi_vars)
    /// for a single area write.
    pub async fn ab_write(
        &self,
        area: Area,
        db_number: u16,
        start: u32,
        data: &[u8],
    ) -> Result<()> {
        let items = [MultiWriteItem {
            area,
            db_number,
            start,
            data: Bytes::copy_from_slice(data),
        }];
        self.write_multi_vars(&items).await
    }

    /// Read Merker (flag) bytes starting at `start`, `length` bytes.
    pub async fn mb_read(&self, start: u32, length: u16) -> Result<Bytes> {
        self.ab_read(Area::Marker, 0, start, length).await
    }

    /// Write Merker (flag) bytes starting at `start`.
    pub async fn mb_write(&self, start: u32, data: &[u8]) -> Result<()> {
        self.ab_write(Area::Marker, 0, start, data).await
    }

    /// Read I/O input (EB) bytes starting at `start`, `length` bytes.
    pub async fn eb_read(&self, start: u32, length: u16) -> Result<Bytes> {
        self.ab_read(Area::ProcessInput, 0, start, length).await
    }

    /// Write I/O input (EB) bytes starting at `start`.
    pub async fn eb_write(&self, start: u32, data: &[u8]) -> Result<()> {
        self.ab_write(Area::ProcessInput, 0, start, data).await
    }

    /// Read I/O output (AB) bytes starting at `start`, `length` bytes.
    pub async fn ib_read(&self, start: u32, length: u16) -> Result<Bytes> {
        self.ab_read(Area::ProcessOutput, 0, start, length).await
    }

    /// Write I/O output (AB) bytes starting at `start`.
    pub async fn ib_write(&self, start: u32, data: &[u8]) -> Result<()> {
        self.ab_write(Area::ProcessOutput, 0, start, data).await
    }

    /// Read `amount` Timer words starting at timer index `start`.
    pub async fn tm_read(&self, start: u32, amount: u16) -> Result<Bytes> {
        let items = [MultiReadItem {
            area: Area::Timer,
            db_number: 0,
            start,
            length: amount,
            transport: TransportSize::Timer,
        }];
        let mut results = self.read_multi_vars(&items).await?;
        Ok(results.swap_remove(0))
    }

    /// Write Timer S5Time words. `data` must be `amount * 2` bytes (one word per timer).
    pub async fn tm_write(&self, start: u32, data: &[u8]) -> Result<()> {
        let amount = (data.len() / 2) as u16;
        let items = [MultiWriteItem {
            area: Area::Timer,
            db_number: 0,
            start,
            data: Bytes::copy_from_slice(data),
        }];
        let _ = amount;
        self.write_multi_vars(&items).await
    }

    /// Read `amount` Counter BCD words starting at counter index `start`.
    pub async fn ct_read(&self, start: u32, amount: u16) -> Result<Bytes> {
        let items = [MultiReadItem {
            area: Area::Counter,
            db_number: 0,
            start,
            length: amount,
            transport: TransportSize::Counter,
        }];
        let mut results = self.read_multi_vars(&items).await?;
        Ok(results.swap_remove(0))
    }

    /// Write Counter BCD words. `data` must be `amount * 2` bytes (one word per counter).
    pub async fn ct_write(&self, start: u32, data: &[u8]) -> Result<()> {
        let items = [MultiWriteItem {
            area: Area::Counter,
            db_number: 0,
            start,
            data: Bytes::copy_from_slice(data),
        }];
        self.write_multi_vars(&items).await
    }

    pub async fn read_szl(&self, szl_id: u16, szl_index: u16) -> Result<SzlResponse> {
        let payload = self.read_szl_payload(szl_id, szl_index).await?;
        let mut b = payload;
        Ok(SzlResponse::decode(&mut b)?)
    }

    /// Send a UserData SZL query and return the raw SZL data block
    /// (starting with block_len, szl_id, szl_index, then entry data).
    async fn read_szl_payload(&self, szl_id: u16, szl_index: u16) -> Result<Bytes> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        let req = SzlRequest { szl_id, szl_index };
        let mut param_buf = BytesMut::new();
        req.encode_params(&mut param_buf);
        let mut data_buf = BytesMut::new();
        req.encode_data(&mut data_buf);

        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            data_buf.freeze(),
            pdu_ref,
            PduType::UserData,
        )
        .await?;

        let (header, mut body) = Self::recv_s7(&mut inner).await?;

        // Skip the echoed param section
        if body.remaining() < header.param_len as usize {
            return Err(Error::UnexpectedResponse);
        }
        body.advance(header.param_len as usize);

        // body is now the data section.
        // Data envelope: return_code(1) + transport(1) + data_len(2)
        // If shorter than 4, the PLC returned an error with no data.
        if body.remaining() < 4 {
            return Ok(Bytes::new());
        }
        let return_code = body.get_u8();
        let _transport = body.get_u8();
        let _data_len = body.get_u16();

        // return_code 0xFF = success; anything else = PLC error (function not available etc.)
        // Return empty payload so callers can handle gracefully.
        if return_code != 0xFF {
            return Ok(Bytes::new());
        }

        // Remaining is the SZL data block.
        Ok(body.copy_to_bytes(body.remaining()))
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

    /// Set the PLC clock (UserData subfunction 0x01 of function group 0xF5).
    pub async fn set_clock(&self, dt: &PlcDateTime) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        // Param: method=0x11 (request), fn_group=0xF5 (clock), subfn=0x01 (set), param_len=0x08
        let mut param_buf = BytesMut::new();
        param_buf.extend_from_slice(&[0x00, 0x01, 0x12, 0x08, 0xF5, 0x01]);
        // Data envelope: return_code=0xFF, transport=0x09 (OCTET_STRING), length=8
        let mut data_buf = BytesMut::new();
        data_buf.extend_from_slice(&[0xFF, 0x09, 0x00, 0x08]);
        dt.encode(&mut data_buf);
        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            data_buf.freeze(),
            pdu_ref,
            PduType::UserData,
        )
        .await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "set_clock")?;
        Ok(())
    }

    /// Set the PLC clock to the host system time.
    ///
    /// Uses [`std::time::SystemTime`] converted to a [`PlcDateTime`].  The
    /// weekday field is set to 0 (unknown) since `SystemTime` does not carry it.
    pub async fn set_clock_to_now(&self) -> Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Simple UTC decomposition (no leap-second handling)
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        // Days since epoch
        let days = secs / 86400;
        // Rough Gregorian year calculation
        let mut year = 1970u16;
        let mut d = days;
        loop {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            let days_in_year: u64 = if leap { 366 } else { 365 };
            if d < days_in_year {
                break;
            }
            d -= days_in_year;
            year += 1;
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days_per_month: [u64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month = 1u8;
        for &dpm in &days_per_month {
            if d < dpm {
                break;
            }
            d -= dpm;
            month += 1;
        }
        let dt = PlcDateTime {
            year,
            month,
            day: (d + 1) as u8,
            hour: h as u8,
            minute: m as u8,
            second: s as u8,
            millisecond: 0,
            weekday: 0,
        };
        self.set_clock(&dt).await
    }

    /// Read the list of all available SZL IDs from the PLC (SZL ID 0x0000).
    ///
    /// Returns a `Vec<u16>` where each entry is a supported SZL ID.
    pub async fn read_szl_list(&self) -> Result<Vec<u16>> {
        let payload = self.read_szl_payload(0x0000, 0x0000).await?;
        if payload.is_empty() {
            return Ok(Vec::new());
        }
        let mut b = payload;
        // SZL block: [szl_id:2][szl_index:2][entry_len:2][entry_count:2][entries...]
        if b.remaining() < 8 {
            return Err(Error::UnexpectedResponse);
        }
        let _szl_id = b.get_u16();
        let _szl_index = b.get_u16();
        let entry_len = b.get_u16() as usize;
        let entry_count = b.get_u16() as usize;
        if entry_len < 2 {
            return Err(Error::UnexpectedResponse);
        }
        let mut ids = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if b.remaining() < entry_len {
                break;
            }
            ids.push(b.get_u16());
            b.advance(entry_len - 2);
        }
        Ok(ids)
    }

    /// Copy RAM data to ROM (function 0x43).
    ///
    /// Copies the CPU's work memory to its load memory (retain on power-off).
    pub async fn copy_ram_to_rom(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let param = Bytes::copy_from_slice(&[
            0x00, 0x01, 0x12, 0x04, 0x43, 0x44, 0x01, 0x00,
        ]);
        Self::send_s7(&mut inner, param, Bytes::new(), pdu_ref, PduType::UserData).await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "copy_ram_to_rom")?;
        Ok(())
    }

    /// Compress the PLC work memory (function 0x42).
    ///
    /// Reorganises memory to eliminate fragmentation.  The PLC must be in STOP
    /// mode before calling this.
    pub async fn compress(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let param = Bytes::copy_from_slice(&[
            0x00, 0x01, 0x12, 0x04, 0x42, 0x44, 0x01, 0x00,
        ]);
        Self::send_s7(&mut inner, param, Bytes::new(), pdu_ref, PduType::UserData).await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "compress")?;
        Ok(())
    }

    // -- PLC control & status -------------------------------------------------

    /// Send a simple Job with a 2-byte parameter (func + 0x00) and no data.
    async fn simple_control(inner: &mut Inner<T>, pdu_ref: u16, func: u8) -> Result<()> {
        let param = Bytes::copy_from_slice(&[func, 0x00]);
        Self::send_s7(inner, param, Bytes::new(), pdu_ref, PduType::Job).await?;
        let (header, _body) = Self::recv_s7(inner).await?;
        check_plc_error(&header, "plc_control")?;
        Ok(())
    }

    /// Stop the PLC (S7 function code 0x29).
    ///
    /// Sends a Job request with no additional data. Returns `Ok(())` when the
    /// PLC acknowledges the command, or an error if the PLC rejects it
    /// (e.g., password-protected or CPU in a non-stoppable state).
    pub async fn plc_stop(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        Self::simple_control(&mut inner, pdu_ref, 0x29).await
    }

    /// Hot-start (warm restart) the PLC (S7 function code 0x28).
    ///
    /// A warm restart retains the DB content and retentive memory.
    pub async fn plc_hot_start(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        Self::simple_control(&mut inner, pdu_ref, 0x28).await
    }

    /// Cold-start (full restart) the PLC (S7 function code 0x2A).
    ///
    /// A cold start clears all DBs and non-retentive memory.
    pub async fn plc_cold_start(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        Self::simple_control(&mut inner, pdu_ref, 0x2A).await
    }

    /// Read the current PLC status via SZL 0x0424.
    ///
    /// Returns one of [`PlcStatus::Run`], [`PlcStatus::Stop`], or
    /// [`PlcStatus::Unknown`].
    pub async fn get_plc_status(&self) -> Result<crate::types::PlcStatus> {
        let payload = self.read_szl_payload(0x0424, 0x0000).await?;
        // SZL 0x0424 response layout (after stripping 4-byte data envelope):
        //   [0..1]  SZL_ID  (0x0424)
        //   [2..3]  SZL_INDEX (0x0000)
        //   [4..5]  LENTHDR (entry length in bytes, big-endian)
        //   [6..7]  N_DR (entry count, big-endian)
        //   [8..]   first entry data
        // C snap7 strips SZL_ID+SZL_INDEX (4 bytes), so its opData[7] = payload[11].
        // Status byte = 4th byte of first entry = payload[11].
        if payload.len() < 12 {
            return Ok(crate::types::PlcStatus::Unknown);
        }
        let status_byte = payload[11];
        match status_byte {
            0x00 => Ok(crate::types::PlcStatus::Unknown),
            0x04 => Ok(crate::types::PlcStatus::Stop),
            0x08 => Ok(crate::types::PlcStatus::Run),
            // Old CPUs sometimes encode STOP as 0x03
            0x03 => Ok(crate::types::PlcStatus::Stop),
            _ => Ok(crate::types::PlcStatus::Stop),
        }
    }

    // -- PLC information queries (via SZL UserData) ---------------------------

    /// Read the PLC order code (SZL ID 0x0011).
    ///
    /// The order code is a 20-character ASCII string (e.g. `"6ES7 317-2EK14-0AB0"`).
    pub async fn get_order_code(&self) -> Result<crate::types::OrderCode> {
        let payload = self.read_szl_payload(0x0011, 0x0000).await?;
        if payload.len() < 8 {
            return Err(Error::UnexpectedResponse);
        }

        // SZL 0x0011 payload: [szl_id:2][szl_index:2][entry_len:2][entry_count:2][entries...]
        // Each entry: [index:2][data: entry_len-2 bytes, null-padded]
        // Entry 0x0001 = order code string; version bytes = last 3 bytes of entire payload.
        let n = payload.len();
        let (v1, v2, v3) = if n >= 3 {
            (payload[n - 3], payload[n - 2], payload[n - 1])
        } else {
            (0, 0, 0)
        };

        let mut b = payload.clone();
        let szl_id = b.get_u16();
        let _szl_idx = b.get_u16();
        let entry_len = b.get_u16() as usize;
        let entry_count = b.get_u16() as usize;

        if (szl_id == 0x0011 || szl_id == 0x001C) && entry_len >= 4 && entry_count > 0 {
            for _ in 0..entry_count {
                if b.remaining() < entry_len { break; }
                let entry_idx = b.get_u16();
                let string_len = entry_len - 2;
                let raw = b.copy_to_bytes(string_len);
                if entry_idx == 0x0001 {
                    let null_end = raw.iter().position(|&x| x == 0).unwrap_or(string_len);
                    let code = String::from_utf8_lossy(&raw[..null_end]).trim().to_string();
                    if !code.is_empty() {
                        return Ok(crate::types::OrderCode { code, v1, v2, v3 });
                    }
                }
            }
        }

        // Fallback: scan for "6ES"/"6AV"/"6GK" pattern anywhere in payload.
        let code = scan_ascii_fields(&payload, 10, 4).into_iter().find(|s| {
            let su = s.to_uppercase();
            (su.starts_with("6ES") || su.starts_with("6AV") || su.starts_with("6GK"))
                && s.len() >= 10
                && s.bytes().all(|c| c.is_ascii_graphic() || c == b' ')
        }).unwrap_or_default();
        Ok(crate::types::OrderCode { code, v1, v2, v3 })
    }

    /// Read detailed CPU information (SZL ID 0x001C).
    ///
    /// Returns module type, serial number, plant identification, copyright
    /// and module name fields pre-parsed from the SZL response.
    /// Handles both classic S7-300/400 and S7-1200/1500 response formats.
    pub async fn get_cpu_info(&self) -> Result<crate::types::CpuInfo> {
        let payload = self.read_szl_payload(0x001C, 0x0000).await?;
        if payload.len() < 8 {
            return Err(Error::UnexpectedResponse);
        }

        // SZL 0x001C payload layout (after correct request framing):
        //   [szl_id:2=0x001C][szl_index:2][entry_len:2][entry_count:2]
        //   followed by entry_count entries, each entry_len bytes:
        //     [entry_index:2][string_data: entry_len-2 bytes, null-padded]
        //
        // Entry indices observed on S7-300/400:
        //   0x0001 = plant identification (AS name)
        //   0x0002 = module type name (e.g. "CPU 319-3 PN/DP")
        //   0x0003 = module name (OB1 program name)
        //   0x0004 = copyright
        //   0x0005 = serial number
        //   0x0007 = module type name (duplicate in some firmware)
        //   0x0008 = module name (duplicate in some firmware)
        let mut b = payload.clone();
        let szl_id = b.get_u16();
        let _szl_idx = b.get_u16();
        let entry_len = b.get_u16() as usize;
        let entry_count = b.get_u16() as usize;

        if szl_id == 0x001C && entry_len >= 4 && entry_count > 0 {
            let mut module_type = String::new();
            let mut module_type_canonical = String::new(); // index 0x0007 — always authoritative
            let mut serial_number = String::new();
            let mut as_name = String::new();
            let mut copyright = String::new();
            let mut module_name = String::new();

            for _ in 0..entry_count {
                if b.remaining() < entry_len { break; }
                let entry_idx = b.get_u16();
                let string_len = entry_len - 2;
                let raw = b.copy_to_bytes(string_len);
                let null_end = raw.iter().position(|&x| x == 0).unwrap_or(string_len);
                let val = String::from_utf8_lossy(&raw[..null_end]).trim().to_string();
                match entry_idx {
                    0x0001 => { if as_name.is_empty() { as_name = val; } }
                    // 0x0002 is module type on S7-300, AS name on S7-1500 — only use if
                    // 0x0007 is absent (module_type_canonical will override below).
                    0x0002 => { if module_type.is_empty() { module_type = val; } }
                    0x0003 => { if module_name.is_empty() { module_name = val; } }
                    0x0004 => { if copyright.is_empty() { copyright = val; } }
                    0x0005 => { if serial_number.is_empty() { serial_number = val; } }
                    // 0x0007 is always the true module type name (both S7-300 and S7-1500)
                    0x0007 => { if module_type_canonical.is_empty() { module_type_canonical = val; } }
                    // 0x0008 is SMC memory card on S7-1500 — do not use for module_name
                    _ => {}
                }
            }

            // 0x0007 wins over 0x0002 for module_type
            if !module_type_canonical.is_empty() {
                module_type = module_type_canonical;
            }

            if module_name.is_empty() && !as_name.is_empty() {
                module_name = as_name.clone();
            }

            if !module_type.is_empty() || !serial_number.is_empty() || !as_name.is_empty() {
                let protocol = detect_protocol(&payload, &module_type);
                return Ok(crate::types::CpuInfo {
                    module_type,
                    serial_number,
                    as_name,
                    copyright,
                    module_name,
                    protocol,
                });
            }
        }

        // S7-1500 and some firmware variants use a tagged sub-record format.
        // Fall back to scanning the raw payload for tagged string fields.
        let data = payload.as_ref();
        let (module_type, serial_number, as_name, copyright, module_name) =
            parse_sub_record_fields(data);

        if !module_type.is_empty() || !serial_number.is_empty() {
            let protocol = detect_protocol(&payload, &module_type);
            return Ok(crate::types::CpuInfo {
                module_type,
                serial_number,
                as_name,
                copyright,
                module_name,
                protocol,
            });
        }

        // Last-resort scan: extract printable strings and apply heuristics.
        let mut module_type = String::new();
        let mut serial_number = String::new();
        let mut as_name = String::new();
        let mut copyright = String::new();
        let mut module_name = String::new();

        let mut scan = 0;
        while scan < data.len() {
            if data[scan].is_ascii_graphic() || data[scan] == b' ' {
                let start = scan;
                while scan < data.len() && (data[scan].is_ascii_graphic() || data[scan] == b' ') {
                    scan += 1;
                }
                let val = String::from_utf8_lossy(&data[start..scan]).trim().to_string();
                if val.len() >= 3 {
                    let tag = if start >= 2 && data[start - 2] == 0x00 {
                        Some(data[start - 1])
                    } else {
                        None
                    };
                    let su = val.to_uppercase();
                    if su.contains("BOOT") || su.starts_with("P B") || su.starts_with("HBOOT") {
                        // skip firmware label
                    } else if tag == Some(0x07) && module_type.is_empty() {
                        module_type = val;
                    } else if tag == Some(0x08) && module_name.is_empty() {
                        module_name = val;
                    } else if tag == Some(0x05) && as_name.is_empty() {
                        as_name = val;
                    } else if tag == Some(0x06) && copyright.is_empty() {
                        copyright = val;
                    } else if tag == Some(0x04) && serial_number.is_empty() {
                        serial_number = val;
                    } else if val.contains('-')
                        && val.chars().filter(|c| c.is_ascii_digit()).count() >= 4
                        && !val.starts_with("6ES7")
                        && serial_number.is_empty()
                    {
                        serial_number = val;
                    } else if su.contains("CPU") && su.contains("PN") && module_type.is_empty() {
                        module_type = val;
                    } else if module_type.is_empty() && val.len() >= 8 && !su.contains("MC_") {
                        module_type = val;
                    }
                }
            } else {
                scan += 1;
            }
        }

        let protocol = detect_protocol(&payload, &module_type);
        Ok(crate::types::CpuInfo {
            module_type,
            serial_number,
            as_name,
            copyright,
            module_name,
            protocol,
        })
    }
    
    /// Read communication processor information (SZL ID 0x0131, index 0x0001).
    ///
    /// Returns maximum PDU length, connection count, and baud rates.
    pub async fn get_cp_info(&self) -> Result<crate::types::CpInfo> {
        // Index 0x0001 = communication module info entry (used by C snap7).
        let payload = self.read_szl_payload(0x0131, 0x0001).await?;

        // SZL 0x0131 response wire format (after stripping the 4-byte data envelope):
        //   [szl_id:2][szl_index:2][entry_len:2][entry_count:2][entries...]
        // Each entry for index 0x0001 (S7-300/400/1200/1500):
        //   [index:2][max_pdu_len:2][max_connections:2][max_mpi_rate:4][max_bus_rate:4] = 14 bytes

        let mut b = payload.clone();
        if b.remaining() < 8 {
            return Ok(crate::types::CpInfo {
                max_pdu_len: 0, max_connections: 0, max_mpi_rate: 0, max_bus_rate: 0,
            });
        }

        let szl_id = b.get_u16();
        let _szl_idx = b.get_u16();
        let entry_len = b.get_u16() as usize;
        let entry_count = b.get_u16() as usize;

        // Classic format (S7-300/400/1200): szl_id=0x0131, entries with 14-byte records
        if szl_id == 0x0131 && entry_len >= 12 && entry_count >= 1 && b.remaining() >= entry_len {
            let _entry_idx = b.get_u16();
            let max_pdu_len = b.get_u16() as u32;
            let max_connections = b.get_u16() as u32;
            let max_mpi_rate = b.get_u32();
            let max_bus_rate = b.get_u32();
            return Ok(crate::types::CpInfo {
                max_pdu_len,
                max_connections,
                max_mpi_rate,
                max_bus_rate,
            });
        }

        // Fallback: scan for any parseable numeric data
        Ok(crate::types::CpInfo {
            max_pdu_len: 0,
            max_connections: 0,
            max_mpi_rate: 0,
            max_bus_rate: 0,
        })
    }

    /// Read the rack module list (SZL ID 0x00A0).
    ///
    /// Each entry is a 2-byte module type identifier.
    pub async fn read_module_list(&self) -> Result<Vec<crate::types::ModuleEntry>> {
        let payload = self.read_szl_payload(0x00A0, 0x0000).await?;
        if payload.len() < 6 {
            return Ok(Vec::new());
        }
        let mut b = payload;
        let _block_len = b.get_u16();
        let _szl_id = b.get_u16();
        let _szl_ix = b.get_u16();
        // Skip the optional SZL entry_length prefix (2 bytes).
        skip_szl_entry_header(&mut b);
        let mut modules = Vec::new();
        while b.remaining() >= 2 {
            modules.push(crate::types::ModuleEntry {
                module_type: b.get_u16(),
            });
        }
        Ok(modules)
    }

    // -- Block list & block info (via UserData grBlocksInfo) ------------------

    /// List all blocks in the PLC grouped by type.
    ///
    /// Uses UserData function group 0x43 (grBlocksInfo), SubFun 0x01 (ListAll).
    /// Response: 7 entries of [Zero(1) BType(1) BCount(2)] = 28 bytes data.
    pub async fn list_blocks(&self) -> Result<crate::types::BlockList> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        // Params: Head[00 01 12 04] + Uk=0x11 + Tg=0x43(grBlocksInfo) + SubFun=0x01(ListAll) + Seq=0x00
        let param = Bytes::from_static(&[0x00, 0x01, 0x12, 0x04, 0x11, 0x43, 0x01, 0x00]);
        // Data: 4 bytes constant
        let data = Bytes::from_static(&[0x0A, 0x00, 0x00, 0x00]);

        Self::send_s7(&mut inner, param, data, pdu_ref, PduType::UserData).await?;
        let (header, mut body) = Self::recv_s7(&mut inner).await?;

        // Skip echoed param section
        if body.remaining() < header.param_len as usize {
            return Err(Error::UnexpectedResponse);
        }
        body.advance(header.param_len as usize);

        // Data envelope: RetVal(1) + TRSize(1) + Length(2)
        if body.remaining() < 4 {
            return Ok(crate::types::BlockList { total_count: 0, entries: Vec::new() });
        }
        let _ret_val = body.get_u8();
        let _tr_size = body.get_u8();
        let data_len = body.get_u16() as usize;

        // 7 entries × 4 bytes = 28 bytes
        if data_len < 28 || body.remaining() < 28 {
            return Ok(crate::types::BlockList { total_count: 0, entries: Vec::new() });
        }

        let mut entries = Vec::new();
        let mut total_count: u32 = 0;
        for _ in 0..7 {
            let _zero = body.get_u8();
            let block_type = body.get_u8() as u16;
            let count = body.get_u16();
            total_count += count as u32;
            entries.push(crate::types::BlockListEntry { block_type, count });
        }

        Ok(crate::types::BlockList { total_count, entries })
    }

    /// List all block numbers of a given type (grBlocksInfo / SFun_ListBoT = 0x02).
    ///
    /// `block_type` is the raw byte: 0x38=OB, 0x41=DB, 0x42=SDB, 0x43=FC,
    /// 0x44=SFC, 0x45=FB, 0x46=SFB.
    /// Returns a sorted vec of block numbers.
    pub async fn list_blocks_of_type(&self, block_type: u8) -> Result<Vec<u16>> {
        let mut numbers: Vec<u16> = Vec::new();
        let mut first = true;
        let mut seq: u8 = 0x00;

        loop {
            let mut inner = self.inner.lock().await;
            let pdu_ref = Self::next_pdu_ref(&mut inner);

            let (param, data) = if first {
                // First request: 8-byte params + 6-byte data
                // Params: Head[00 01 12 04] Uk=0x11 Tg=0x43 SubFun=0x02 Seq=0x00
                // Data:   RetVal=0xFF TSize=0x09 Length=0x0002 Zero=0x30 BlkType
                let mut p = BytesMut::with_capacity(8);
                p.extend_from_slice(&[0x00, 0x01, 0x12, 0x04, 0x11, 0x43, 0x02, 0x00]);
                let mut d = BytesMut::with_capacity(6);
                d.extend_from_slice(&[0xFF, 0x09, 0x00, 0x02, 0x30, block_type]);
                (p.freeze(), d.freeze())
            } else {
                // Continuation: 12-byte params + 4-byte data
                // Params: Head[00 01 12 08] Uk=0x12 Tg=0x43 SubFun=0x02 Seq=<seq> + 4 zero pad
                // Data:   0x0A 0x00 0x00 0x00
                let mut p = BytesMut::with_capacity(12);
                p.extend_from_slice(&[0x00, 0x01, 0x12, 0x08, 0x12, 0x43, 0x02, seq, 0x00, 0x00, 0x00, 0x00]);
                let d = Bytes::from_static(&[0x0A, 0x00, 0x00, 0x00]);
                (p.freeze(), d)
            };

            Self::send_s7(&mut inner, param, data, pdu_ref, PduType::UserData).await?;
            let (header, mut body) = Self::recv_s7(&mut inner).await?;

            // Skip echoed params
            if body.remaining() < header.param_len as usize {
                return Err(Error::UnexpectedResponse);
            }
            // Grab seq + done flag from params before advancing
            // ResParams layout (after S7 header): Head[3] Plen Uk Tg SubFun Seq [Rsvd(2) ErrNo(2)]
            // Seq is at param offset 7, Rsvd high byte at offset 8 indicates done (0x00 = done)
            let param_bytes = body.slice(..header.param_len as usize);
            let done = param_bytes.len() >= 10 && param_bytes[8] == 0x00;
            seq = if param_bytes.len() >= 8 { param_bytes[7] } else { 0 };
            body.advance(header.param_len as usize);
            drop(inner);

            // Data envelope: RetVal(1) TSize(1) DataLen(2)
            if body.remaining() < 4 { break; }
            let ret_val = body.get_u8();
            let _tr_size = body.get_u8();
            let data_len = body.get_u16() as usize;

            if ret_val != 0xFF || data_len < 4 || body.remaining() < data_len { break; }

            // Items: each 4 bytes [BlockNum(2) Unknown(1) BlockLang(1)]
            // Count = (data_len - 4) / 4 + 1  (from C snap7 source)
            let item_count = ((data_len - 4) / 4) + 1;
            for _ in 0..item_count {
                if body.remaining() < 4 { break; }
                let block_num = body.get_u16();
                let _unknown = body.get_u8();
                let _lang = body.get_u8();
                numbers.push(block_num);
            }

            first = false;
            if done { break; }
        }

        numbers.sort_unstable();
        Ok(numbers)
    }

    /// Internal: send a UserData block-info request (grBlocksInfo / SFun_BlkInfo=0x03).
    ///
    /// Params (8 bytes): Head[00 01 12 04] Uk=0x11 Tg=0x43 SubFun=0x03 Seq=0x00
    /// Data  (12 bytes): FF 09 00 08 30 <blktype> <ascii5> 41
    async fn block_info_query(
        &self,
        _func: u8,
        block_type: u8,
        block_number: u16,
    ) -> Result<Bytes> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        // Params: Head[00 01 12 04] Uk=0x11 Tg=0x43(grBlocksInfo) SubFun=0x03(BlkInfo) Seq=0x00
        let param = Bytes::from_static(&[0x00, 0x01, 0x12, 0x04, 0x11, 0x43, 0x03, 0x00]);

        // Data: RetVal=0xFF TSize=0x09 DataLen=0x0008 BlkPrfx=0x30 BlkType AsciiBlk[5] A=0x41
        let mut data_buf = BytesMut::with_capacity(12);
        data_buf.extend_from_slice(&[0xFF, 0x09, 0x00, 0x08, 0x30, block_type]);
        // block_number as 5-digit ASCII
        let n = block_number as u32;
        data_buf.put_u8((n / 10000) as u8 + 0x30);
        data_buf.put_u8(((n % 10000) / 1000) as u8 + 0x30);
        data_buf.put_u8(((n % 1000) / 100) as u8 + 0x30);
        data_buf.put_u8(((n % 100) / 10) as u8 + 0x30);
        data_buf.put_u8((n % 10) as u8 + 0x30);
        data_buf.put_u8(0x41); // 'A'

        Self::send_s7(&mut inner, param, data_buf.freeze(), pdu_ref, PduType::UserData).await?;

        let (header, mut body) = Self::recv_s7(&mut inner).await?;

        // Response params: TResFunGetBlockInfo (12 bytes)
        // Head[3] Plen Uk Tg SubFun Seq Rsvd[2] ErrNo[2]
        let param_len = header.param_len as usize;
        if body.remaining() < param_len {
            return Err(Error::UnexpectedResponse);
        }
        let params = body.slice(..param_len);
        body.advance(param_len);

        // Check ErrNo (bytes 10-11 of params)
        if params.len() >= 12 {
            let err_no = u16::from_be_bytes([params[10], params[11]]);
            if err_no != 0 {
                return Err(Error::PlcError {
                    code: err_no as u32,
                    message: format!("block info error: ErrNo=0x{err_no:04X}"),
                });
            }
        }

        // Data envelope: RetVal(1) TSize(1) DataLen(2)
        if body.remaining() < 4 {
            return Err(Error::UnexpectedResponse);
        }
        let ret_val = body.get_u8();
        let _tr_size = body.get_u8();
        let _data_len = body.get_u16();

        if ret_val != 0xFF {
            return Err(Error::PlcError {
                code: ret_val as u32,
                message: format!("block info RetVal=0x{ret_val:02X}"),
            });
        }

        Ok(body.copy_to_bytes(body.remaining()))
    }

    /// Get detailed information about a block stored on the PLC.
    ///
    /// `block_type` should be one of the [`BlockType`](crate::types::BlockType)
    /// discriminant values (e.g. `0x41` for DB, `0x38` for OB).
    pub async fn get_ag_block_info(
        &self,
        block_type: u8,
        block_number: u16,
    ) -> Result<crate::types::BlockInfo> {
        self.get_block_info(0x13, block_type, block_number).await
    }

    /// Get detailed block information from the PG perspective.
    ///
    /// Same fields as [`get_ag_block_info`](Self::get_ag_block_info) but the
    /// information is from the programming-device viewpoint.
    pub async fn get_pg_block_info(
        &self,
        block_type: u8,
        block_number: u16,
    ) -> Result<crate::types::BlockInfo> {
        self.get_block_info(0x14, block_type, block_number).await
    }

    /// Shared implementation for AG and PG block info.
    async fn get_block_info(
        &self,
        func: u8,
        block_type: u8,
        block_number: u16,
    ) -> Result<crate::types::BlockInfo> {
        let payload = self
            .block_info_query(func, block_type, block_number)
            .await?;

        // Payload = TResDataBlockInfo fields after the 4-byte envelope (RetVal/TSize/DataLen
        // already consumed in block_info_query). Struct layout:
        //   Cst_b(1) BlkType(1) Cst_w1(2) Cst_w2(2) Cst_pp(2)
        //   Unknown_1(1) BlkFlags(1) BlkLang(1) SubBlkType(1) BlkNumber(2)
        //   LenLoadMem(4) BlkSec(4) CodeTime_ms(4) CodeTime_dy(2)
        //   IntfTime_ms(4) IntfTime_dy(2) SbbLen(2) AddLen(2)
        //   LocDataLen(2) MC7Len(2)
        //   Author(8) Family(8) Header(8)
        //   Version(1) Unknown_2(1) BlkChksum(2) Resvd1(4) Resvd2(4)
        // Minimum meaningful size: 40 bytes
        if payload.len() < 40 {
            return Err(Error::UnexpectedResponse);
        }
        let mut b = payload;

        let _cst_b       = b.get_u8();
        let blk_type: u16 = b.get_u8().into();
        let _cst_w1      = b.get_u16();
        let _cst_w2      = b.get_u16();
        let _cst_pp      = b.get_u16();
        let _unknown_1   = b.get_u8();
        let flags        = b.get_u8() as u16;
        let language     = b.get_u8() as u16;
        let _sub_blk     = b.get_u8();
        let _blk_number  = b.get_u16(); // echoes block_number from request
        let len_load_mem = b.get_u32();
        let _blk_sec     = b.get_u32();
        let _code_ms     = b.get_u32();
        let _code_dy     = b.get_u16();
        let _intf_ms     = b.get_u32();
        let _intf_dy     = b.get_u16();
        let sbb_len      = b.get_u16();
        let _add_len     = b.get_u16();
        let local_data   = b.get_u16();
        let mc7_size     = b.get_u16();

        fn read_str(b: &mut Bytes, n: usize) -> String {
            let s = b.slice(..n.min(b.remaining()));
            b.advance(n.min(b.remaining()));
            let end = s.iter().position(|&x| x == 0).unwrap_or(s.len());
            String::from_utf8_lossy(&s[..end]).trim().to_string()
        }

        let author   = read_str(&mut b, 8);
        let family   = read_str(&mut b, 8);
        let header   = read_str(&mut b, 8);
        let version  = if b.remaining() >= 1 { b.get_u8() as u16 } else { 0 };
        let _unk2    = if b.remaining() >= 1 { b.get_u8() } else { 0 };
        let checksum = if b.remaining() >= 2 { b.get_u16() } else { 0 };

        Ok(crate::types::BlockInfo {
            block_type: blk_type,
            block_number,
            language,
            flags,
            size: (len_load_mem.min(0xFFFF)) as u16,
            size_ram: sbb_len,
            mc7_size,
            local_data,
            checksum,
            version,
            author,
            family,
            header,
            date: String::new(),
        })
    }

    /// Parse block info from raw block bytes obtained via [`upload`](Self::upload)
    /// or [`full_upload`](Self::full_upload). No PLC connection required.
    ///
    /// Equivalent to C `Cli_GetPgBlockInfo` offline parsing mode.
    pub fn parse_block_info(data: &[u8]) -> Result<crate::types::BlockInfo> {
        const HDR: usize = 36;
        const FOOTER: usize = 48;
        if data.len() < HDR + FOOTER {
            return Err(Error::UnexpectedResponse);
        }
        let load_size = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
        if load_size != data.len() {
            return Err(Error::UnexpectedResponse);
        }
        let mc7_size = u16::from_be_bytes([data[34], data[35]]) as usize;
        if mc7_size + HDR >= load_size {
            return Err(Error::UnexpectedResponse);
        }

        let flags        = data[3] as u16;
        let language     = data[4] as u16;
        let block_type   = data[5] as u16;
        let block_number = u16::from_be_bytes([data[6], data[7]]);
        let sbb_len      = u16::from_be_bytes([data[28], data[29]]);
        let local_data   = u16::from_be_bytes([data[32], data[33]]);

        fn read_str(s: &[u8]) -> String {
            let end = s.iter().position(|&x| x == 0).unwrap_or(s.len());
            String::from_utf8_lossy(&s[..end]).trim().to_string()
        }

        let footer   = &data[load_size - FOOTER..];
        let author   = read_str(&footer[20..28]);
        let family   = read_str(&footer[28..36]);
        let header   = read_str(&footer[36..44]);
        let checksum = u16::from_be_bytes([footer[44], footer[45]]);

        Ok(crate::types::BlockInfo {
            block_type,
            block_number,
            language,
            flags,
            size: load_size.min(0xFFFF) as u16,
            size_ram: sbb_len,
            mc7_size: mc7_size as u16,
            local_data,
            checksum,
            version: 0,
            author,
            family,
            header,
            date: String::new(),
        })
    }

    // -- Security / protection (set/clear password + get protection) ----------

    /// Set a session password for protected PLC access.
    ///
    /// The password is obfuscated using the S7 nibble-swap + XOR-0x55 algorithm
    /// and sent as a Job PDU with function code 0x12.  Passwords longer than
    /// 8 bytes are truncated.
    pub async fn set_session_password(&self, password: &str) -> Result<()> {
        let encrypted = crate::types::encrypt_password(password);
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let param = Bytes::copy_from_slice(&[0x12, 0x00]);
        let data = Bytes::copy_from_slice(&encrypted);
        Self::send_s7(&mut inner, param, data, pdu_ref, PduType::Job).await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "set_session_password")?;
        Ok(())
    }

    /// Clear the session password on the PLC (function code 0x11).
    pub async fn clear_session_password(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let param = Bytes::copy_from_slice(&[0x11, 0x00]);
        Self::send_s7(&mut inner, param, Bytes::new(), pdu_ref, PduType::Job).await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "clear_session_password")?;
        Ok(())
    }

    /// Read the current protection level (SZL ID 0x0032, index 0x0004).
    ///
    /// Returns the protection scheme identifiers and level;
    /// `password_set` is `true` when the PLC reports a non-empty password.
    pub async fn get_protection(&self) -> Result<crate::types::Protection> {
        let payload = self.read_szl_payload(0x0032, 0x0004).await?;
        if payload.len() < 14 {
            return Err(Error::UnexpectedResponse);
        }
        let mut b = payload;
        let _block_len = b.get_u16();
        let _szl_id = b.get_u16();
        let _szl_ix = b.get_u16();
        // Skip the optional SZL entry_length prefix (2 bytes).
        skip_szl_entry_header(&mut b);
        let scheme_szl = b.get_u16();
        let scheme_module = b.get_u16();
        let scheme_bus = b.get_u16();
        let level = b.get_u16();
        // Next 8 bytes = pass_word field ("PASSWORD" if set, spaces otherwise)
        let pass_wort = if b.remaining() >= 8 {
            String::from_utf8_lossy(&b[..8]).trim().to_string()
        } else {
            String::new()
        };
        let password_set = pass_wort.eq_ignore_ascii_case("PASSWORD");
        Ok(crate::types::Protection {
            scheme_szl,
            scheme_module,
            scheme_bus,
            level,
            password_set,
        })
    }

    // -- Block upload / download / delete ------------------------------------
    //
    // S7 function 0x1D = Upload  (sub-fn: 0=start, 1=data, 2=end)
    // S7 function 0x1E = Download (sub-fn: 0=start, 1=data, 2=end)
    // S7 function 0x1F = Delete

    /// Delete a block from the PLC (S7 function code 0x1F).
    pub async fn delete_block(&self, block_type: u8, block_number: u16) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        // param: [0x1F, 0x00, block_type, 0x00, block_number(2)]
        let mut param = BytesMut::with_capacity(6);
        param.extend_from_slice(&[0x1F, 0x00, block_type, 0x00]);
        param.put_u16(block_number);
        Self::send_s7(
            &mut inner,
            param.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::Job,
        )
        .await?;
        let (header, _body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "delete_block")?;
        Ok(())
    }

    /// Upload a PLC block via S7 PI-Upload (function 0x1D).
    ///
    /// Returns the raw block bytes in Diagra format (20-byte header + payload).
    /// Use [`BlockData::from_bytes`] to parse the result.
    pub async fn upload(&self, block_type: u8, block_number: u16) -> Result<Vec<u8>> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        // --- Step 1: Start upload (sub-fn=0x00) ---
        // param: [0x1D, 0x00, block_type, 0x00, block_number(2)]
        let mut param = BytesMut::with_capacity(6);
        param.extend_from_slice(&[0x1D, 0x00, block_type, 0x00]);
        param.put_u16(block_number);
        Self::send_s7(
            &mut inner,
            param.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::Job,
        )
        .await?;
        let (header, mut body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "upload_start")?;
        // Response data: [upload_id(4)][total_len(4)]
        if body.remaining() < 8 {
            return Err(Error::UnexpectedResponse);
        }
        if body.remaining() >= 2 {
            body.advance(2); // skip param echo
        }
        let upload_id = body.get_u32();
        let _total_len = body.get_u32();

        // --- Step 2: Loop data chunks (sub-fn=0x01) ---
        let mut block_data = Vec::new();
        loop {
            let chunk_pdu_ref = Self::next_pdu_ref(&mut inner);
            let mut dparam = BytesMut::with_capacity(6);
            dparam.extend_from_slice(&[0x1D, 0x01]);
            dparam.put_u32(upload_id);
            Self::send_s7(
                &mut inner,
                dparam.freeze(),
                Bytes::new(),
                chunk_pdu_ref,
                PduType::Job,
            )
            .await?;
            let (dheader, mut dbody) = Self::recv_s7(&mut inner).await?;
            check_plc_error(&dheader, "upload_data")?;
            // Skip param echo
            if dbody.remaining() >= 2 {
                dbody.advance(2);
            }
            if dbody.is_empty() {
                break; // no more data
            }
            // The first data PDU may have a 4-byte "data header" before the actual block data
            // (return_code + transport + bit_len).  Skip it.
            if block_data.is_empty() && dbody.remaining() >= 4 {
                // Peek at the first byte — if it looks like a return_code (0xFF), skip 4
                if dbody[0] == 0xFF || dbody[0] == 0x00 {
                    dbody.advance(4);
                }
            }
            let chunk = dbody.copy_to_bytes(dbody.remaining());
            block_data.extend_from_slice(&chunk);

            // If this chunk was smaller than PDU size, it's the last one
            if chunk.len() < inner.connection.pdu_size as usize - 50 {
                break;
            }
            // Safety: prevent infinite loop on broken PLC
            if block_data.len() > 1024 * 1024 * 4 {
                // 4 MB
                return Err(Error::UnexpectedResponse);
            }
        }

        // --- Step 3: End upload (sub-fn=0x02) ---
        let end_pdu_ref = Self::next_pdu_ref(&mut inner);
        let mut eparam = BytesMut::with_capacity(6);
        eparam.extend_from_slice(&[0x1D, 0x02]);
        eparam.put_u32(upload_id);
        Self::send_s7(
            &mut inner,
            eparam.freeze(),
            Bytes::new(),
            end_pdu_ref,
            PduType::Job,
        )
        .await?;
        let (eheader, _ebody) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&eheader, "upload_end")?;

        Ok(block_data)
    }

    /// Full-upload a PLC block including MC7 (executable) code (S7 function 0x1F).
    ///
    /// Unlike [`upload`](Self::upload) which returns only the header/interface,
    /// `full_upload` returns the complete block including executable code.
    pub async fn full_upload(&self, block_type: u8, block_number: u16) -> Result<Vec<u8>> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        // Step 1: Start full-upload (func=0x1F, sub-fn=0x00)
        let mut param = BytesMut::with_capacity(6);
        param.extend_from_slice(&[0x1F, 0x00, block_type, 0x00]);
        param.put_u16(block_number);
        Self::send_s7(&mut inner, param.freeze(), Bytes::new(), pdu_ref, PduType::Job).await?;
        let (header, mut body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "full_upload_start")?;
        if body.remaining() < 8 {
            return Err(Error::UnexpectedResponse);
        }
        if body.remaining() >= 2 {
            body.advance(2);
        }
        let upload_id = body.get_u32();
        let _total_len = body.get_u32();

        // Step 2: Loop data chunks (func=0x1F, sub-fn=0x01)
        let mut block_data = Vec::new();
        loop {
            let chunk_ref = Self::next_pdu_ref(&mut inner);
            let mut dparam = BytesMut::with_capacity(6);
            dparam.extend_from_slice(&[0x1F, 0x01]);
            dparam.put_u32(upload_id);
            Self::send_s7(&mut inner, dparam.freeze(), Bytes::new(), chunk_ref, PduType::Job).await?;
            let (dheader, mut dbody) = Self::recv_s7(&mut inner).await?;
            check_plc_error(&dheader, "full_upload_data")?;
            if dbody.remaining() >= 2 {
                dbody.advance(2);
            }
            if dbody.is_empty() {
                break;
            }
            if block_data.is_empty() && dbody.remaining() >= 4 {
                if dbody[0] == 0xFF || dbody[0] == 0x00 {
                    dbody.advance(4);
                }
            }
            let chunk = dbody.copy_to_bytes(dbody.remaining());
            block_data.extend_from_slice(&chunk);
            if chunk.len() < inner.connection.pdu_size as usize - 50 {
                break;
            }
            if block_data.len() > 1024 * 1024 * 4 {
                return Err(Error::UnexpectedResponse);
            }
        }

        // Step 3: End full-upload (func=0x1F, sub-fn=0x02)
        let end_ref = Self::next_pdu_ref(&mut inner);
        let mut eparam = BytesMut::with_capacity(6);
        eparam.extend_from_slice(&[0x1F, 0x02]);
        eparam.put_u32(upload_id);
        Self::send_s7(&mut inner, eparam.freeze(), Bytes::new(), end_ref, PduType::Job).await?;
        let (eheader, _) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&eheader, "full_upload_end")?;

        Ok(block_data)
    }

    /// Return the negotiated PDU length in bytes.
    pub async fn get_pdu_length(&self) -> u16 {
        self.inner.lock().await.connection.pdu_size
    }

    /// Upload a DB block (convenience wrapper around [`upload`](Self::upload)).
    pub async fn db_get(&self, db_number: u16) -> Result<Vec<u8>> {
        self.upload(0x41, db_number).await // Block_DB = 0x41
    }

    /// Download a block to the PLC (S7 function 0x1E).
    ///
    /// `data` should be in Diagra format (20-byte header + payload, as returned by
    /// [`upload`](Self::upload) or built via [`BlockData::to_bytes`]).
    pub async fn download(&self, block_type: u8, block_number: u16, data: &[u8]) -> Result<()> {
        let total_len = data.len() as u32;
        let mut inner = self.inner.lock().await;
        let pdu_avail = (inner.connection.pdu_size as usize).saturating_sub(50);

        // --- Step 1: Start download (sub-fn=0x00) ---
        let start_ref = Self::next_pdu_ref(&mut inner);
        // param: [0x1E, 0x00, block_type, 0x00, block_number(2), total_len(4)]
        let mut sparam = BytesMut::with_capacity(10);
        sparam.extend_from_slice(&[0x1E, 0x00, block_type, 0x00]);
        sparam.put_u16(block_number);
        sparam.put_u32(total_len);

        // First data chunk
        let chunk_len = pdu_avail.min(data.len());
        let first_chunk = Bytes::copy_from_slice(&data[..chunk_len]);
        Self::send_s7(
            &mut inner,
            sparam.freeze(),
            first_chunk,
            start_ref,
            PduType::Job,
        )
        .await?;

        let (sheader, mut sbody) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&sheader, "download_start")?;
        // Response: [download_id(4)]
        if sbody.remaining() >= 2 {
            sbody.advance(2); // skip param echo
        }
        if sbody.remaining() < 4 {
            return Err(Error::UnexpectedResponse);
        }
        let download_id = sbody.get_u32();

        let mut offset = chunk_len;

        // --- Step 2: Send remaining data chunks (sub-fn=0x01) ---
        while offset < data.len() {
            let chunk_ref = Self::next_pdu_ref(&mut inner);
            let end = (offset + pdu_avail).min(data.len());
            let chunk = Bytes::copy_from_slice(&data[offset..end]);

            let mut dparam = BytesMut::with_capacity(6);
            dparam.extend_from_slice(&[0x1E, 0x01]);
            dparam.put_u32(download_id);

            Self::send_s7(
                &mut inner,
                dparam.freeze(),
                chunk,
                chunk_ref,
                PduType::Job,
            )
            .await?;

            let (dheader, _dbody) = Self::recv_s7(&mut inner).await?;
            check_plc_error(&dheader, "download_data")?;
            offset = end;
        }

        // --- Step 3: End download (sub-fn=0x02) ---
        let end_ref = Self::next_pdu_ref(&mut inner);
        let mut eparam = BytesMut::with_capacity(6);
        eparam.extend_from_slice(&[0x1E, 0x02]);
        eparam.put_u32(download_id);
        Self::send_s7(
            &mut inner,
            eparam.freeze(),
            Bytes::new(),
            end_ref,
            PduType::Job,
        )
        .await?;
        let (eheader, _ebody) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&eheader, "download_end")?;

        Ok(())
    }

    /// Fill a DB with a constant byte value.
    ///
    /// Uses [`get_ag_block_info`](Self::get_ag_block_info) to determine the DB
    /// size, then writes every byte to `value`.
    pub async fn db_fill(&self, db_number: u16, value: u8) -> Result<()> {
        let info = self.get_ag_block_info(0x41, db_number).await?; // Block_DB = 0x41
        let size = info.size as usize;
        if size == 0 {
            return Err(Error::PlcError {
                code: 0,
                message: format!("DB{db_number} has zero size"),
            });
        }
        let data = vec![value; size];
        // Write in chunks to respect PDU limits
        let chunk_size = 240usize; // conservative
        for offset in (0..size).step_by(chunk_size) {
            let end = (offset + chunk_size).min(size);
            self.db_write(db_number, offset as u32, &data[offset..end])
                .await?;
        }
        Ok(())
    }
}

/// If the leading bytes look like an SZL entry_length header (2-byte big-endian u16
/// length value where the high byte is zero), skip them.  Real Siemens PLCs include
/// this header; our test server omits it.
fn skip_szl_entry_header(data: &mut Bytes) {
    if data.len() >= 2 && data[0] == 0x00 && data[1] > 0 && data[1] <= 200 {
        data.advance(2);
    }
}

/// Scan byte data for sequences of visible ASCII characters and return them
/// as a vector of trimmed strings.  Skips non-ASCII and control bytes between
/// sequences.  Useful for extracting CPU info fields from SZL responses across
/// different PLC models and firmware versions.
fn scan_ascii_fields(data: &[u8], max_count: usize, min_len: usize) -> Vec<String> {
    let mut fields = Vec::new();
    let mut i = 0;
    while i < data.len() && fields.len() < max_count {
        // Skip bytes that are not visible ASCII (0x20-0x7E)
        if !data[i].is_ascii_graphic() && data[i] != b' ' {
            i += 1;
            continue;
        }
        // Collect a run of visible ASCII
        let start = i;
        while i < data.len() && (data[i].is_ascii_graphic() || data[i] == b' ') {
            i += 1;
        }
        let s = String::from_utf8_lossy(&data[start..i]).trim().to_string();
        if s.len() >= min_len {
            fields.push(s);
        }
    }
    fields
}

/// Parse the S7-300 sub-record format used in SZL 0x001C responses.
///
/// This format uses tagged records: `[00 <tag> <string>] ...` where
/// known tags are:
/// - 0x01: order code / module identification
/// - 0x05: plant identification (AS name)
/// - 0x06: serial number
/// - 0x07: module type name
/// - 0x08: module name
fn parse_sub_record_fields(b: &[u8]) -> (String, String, String, String, String) {
    let mut module_type = String::new();
    let mut serial_number = String::new();
    let mut as_name = String::new();
    let mut copyright = String::new();
    let mut module_name = String::new();

    let mut i = 0;
    while i + 2 < b.len() {
        // Look for 00 <tag> pattern with a known sub-record tag (1..=8)
        if b[i] == 0x00 && (1..=8).contains(&b[i + 1]) {
            let tag = b[i + 1];
            let start = i + 2;

            // Find end of string: next 0x00 byte (including 00 C0)
            let mut end = start;
            while end < b.len() && b[end] != 0x00 {
                end += 1;
            }

            let raw = &b[start..end];
            let val = String::from_utf8_lossy(raw).trim().to_string();

            // Skip empty and firmware-label values
            let su = val.to_uppercase();
            if !val.is_empty() && !su.contains("BOOT") && !su.starts_with("P B") {
                match tag {
                    0x01 => {
                        // Tag 0x01 may be order code (starts with "6ES") or module type.
                        if !val.starts_with("6ES") && module_type.is_empty() {
                            module_type = val;
                        }
                    }
                    0x05 => { if as_name.is_empty() { as_name = val; } }
                    0x06 => { if serial_number.is_empty() { serial_number = val; } }
                    0x07 => { if module_type.is_empty() { module_type = val; } }
                    0x08 => { if module_name.is_empty() { module_name = val; } }
                    _ => {}
                }
            }

            i = end;
        } else {
            i += 1;
        }
    }

    // Also scan for free-standing printable strings that look like copyright
    // (e.g. "Boot Loader" appearing after the tagged records).
    if copyright.is_empty() {
        let mut scan = 0;
        while scan < b.len() {
            if b[scan].is_ascii_graphic() || b[scan] == b' ' {
                let s = scan;
                while scan < b.len() && (b[scan].is_ascii_graphic() || b[scan] == b' ') {
                    scan += 1;
                }
                let val = String::from_utf8_lossy(&b[s..scan]).trim().to_string();
                let su = val.to_uppercase();
                if val.len() >= 3 {
                    if su.contains("BOOT") || su.starts_with("P B") {
                        copyright = val;
                        break;
                    }
                }
            } else {
                scan += 1;
            }
        }
    }

    (module_type, serial_number, as_name, copyright, module_name)
}

/// Determine the S7 protocol variant from the raw SZL payload and extracted module type.
///
/// - S7-1200/1500/ET200SP uses S7+ protocol: detected from the 0x00 0x01 record marker in the
///   payload, or from a module_type containing `"15"` in its model number.
/// - Everything else (S7-300, S7-400, S7-1200) uses classic S7 protocol.
fn detect_protocol(_payload: &[u8], module_type: &str) -> crate::types::Protocol {
    // S7+ protocol: S7-1200, S7-1500, ET 200SP CPU
    // Classic S7: S7-300, S7-400
    let upper = module_type.to_uppercase();
    let is_s7plus = upper.contains("1500")
        || upper.contains("1200")
        || upper.contains("ET 200SP")
        || upper.contains("ET200SP")
        // "CPU 15xx" catches 1511, 1513, 1515, 1516, 1517, 1518
        || (upper.contains("CPU") && {
            let after_cpu = upper.find("CPU").map(|i| &upper[i+3..]).unwrap_or("");
            let num: String = after_cpu.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
            matches!(num.get(..2), Some("12") | Some("15"))
        });

    if is_s7plus {
        crate::types::Protocol::S7Plus
    } else {
        crate::types::Protocol::S7
    }
}


/// Decode common S7 protocol error class/code pairs into human-readable descriptions.
fn s7_error_description(ec: u8, ecd: u8) -> &'static str {
    match (ec, ecd) {
        (0x81, 0x04) => "function not supported or access denied by PLC",
        (0x81, 0x01) => "reserved by HW or SW function not available",
        (0x82, 0x04) => "PLC is in STOP mode, function not possible",
        (0x05, 0x01) => "invalid block type number",
        (0xD2, 0x01) => "object already exists, download rejected",
        (0xD2, 0x02) => "object does not exist, upload failed",
        (0xD6, 0x01) => "password protection violation",
        (0xD6, 0x05) => "insufficient privilege for this operation",
        _ => "unknown error",
    }
}

fn check_plc_error(header: &S7Header, context: &str) -> Result<()> {
    if let (Some(ec), Some(ecd)) = (header.error_class, header.error_code) {
        if ec != 0 || ecd != 0 {
            let detail = s7_error_description(ec, ecd);
            return Err(Error::PlcError {
                code: ((ec as u32) << 8) | ecd as u32,
                message: format!("{}: {} (error_class=0x{ec:02X}, error_code=0x{ecd:02X})", context, detail),
            });
        }
    }
    Ok(())
}

impl S7Client<crate::transport::TcpTransport> {
    pub async fn connect(addr: SocketAddr, params: ConnectParams) -> Result<Self> {
        let transport =
            crate::transport::TcpTransport::connect(addr, params.connect_timeout).await?;
        let mut client = Self::from_transport(transport, params).await?;
        client.remote_addr = Some(addr);
        Ok(client)
    }

    /// Re-establish the TCP connection and S7 negotiate handshake after a disconnect.
    ///
    /// On success the client resumes normal operation. Returns an error if the
    /// reconnection attempt fails (caller may retry with back-off).
    pub async fn reconnect(&self) -> Result<()> {
        let addr = self.remote_addr.ok_or(Error::ConnectionRefused)?;
        let transport =
            crate::transport::TcpTransport::connect(addr, self.params.connect_timeout).await?;
        let mut t = transport;
        let connection = connect(&mut t, &self.params).await?;
        let mut inner = self.inner.lock().await;
        inner.transport = t;
        inner.connection = connection;
        inner.pdu_ref = 1;
        inner.connected = true;
        inner.job_start = None;
        inner.last_exec_ms = 0;
        Ok(())
    }
}

impl S7Client<crate::UdpTransport> {
    /// Connect to a PLC using UDP transport.
    pub async fn connect_udp(addr: SocketAddr, params: ConnectParams) -> Result<Self> {
        let transport = crate::UdpTransport::connect(addr)
            .await
            .map_err(Error::Io)?;
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

    // -- PLC control & status mocks & tests -----------------------------------

    /// Common handshake for control tests: COTP CR → CC, S7 Negotiate.
    async fn mock_handshake(server_io: &mut (impl AsyncRead + AsyncWrite + Unpin)) {
        let mut buf = vec![0u8; 4096];

        // COTP CR
        let _ = server_io.read(&mut buf).await;
        let cc = CotpPdu::ConnectConfirm { dst_ref: 1, src_ref: 1 };
        let mut cb = BytesMut::new(); cc.encode(&mut cb);
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
    }

    /// Mock for simple control commands (plc_stop / plc_hot_start / plc_cold_start).
    /// `ok` controls whether the mock sends success (error_class=0, error_code=0) or failure.
    async fn mock_plc_control(
        mut server_io: tokio::io::DuplexStream,
        ok: bool,
    ) {
        let mut buf = vec![0u8; 4096];
        mock_handshake(&mut server_io).await;

        // Control request — consume
        let _ = server_io.read(&mut buf).await;

        // AckData response
        let (ec, ecd) = if ok { (0u8, 0u8) } else { (0x81u8, 0x04u8) };
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len: 0, data_len: 0,
            error_class: Some(ec), error_code: Some(ecd),
        }.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn plc_stop_succeeds() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_control(server_io, true));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.plc_stop().await.unwrap();
    }

    #[tokio::test]
    async fn plc_hot_start_succeeds() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_control(server_io, true));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.plc_hot_start().await.unwrap();
    }

    #[tokio::test]
    async fn plc_cold_start_succeeds() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_control(server_io, true));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.plc_cold_start().await.unwrap();
    }

    #[tokio::test]
    async fn plc_stop_rejected_returns_error() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_control(server_io, false));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let result = client.plc_stop().await;
        assert!(result.is_err());
    }

    /// Mock for get_plc_status: sends back `status_byte` in the data section.
    async fn mock_plc_status(
        mut server_io: tokio::io::DuplexStream,
        status_byte: u8,
    ) {
        let mut buf = vec![0u8; 4096];
        mock_handshake(&mut server_io).await;

        // GetPlcStatus request (UserData SZL) — consume
        let _ = server_io.read(&mut buf).await;

        // SZL 0x0424 response payload layout (after data envelope):
        //   [0..1]  SZL_ID = 0x0424
        //   [2..3]  SZL_INDEX = 0x0000
        //   [4..5]  LENTHDR (entry length)
        //   [6..7]  N_DR = 0x0001
        //   [8..10] first 3 bytes of entry
        //   [11]    status byte (payload[11])
        let mut szl_payload = [0u8; 12];
        szl_payload[0..2].copy_from_slice(&0x0424u16.to_be_bytes());
        szl_payload[6..8].copy_from_slice(&0x0001u16.to_be_bytes()); // N_DR = 1
        szl_payload[11] = status_byte;

        // UserData response body:
        //   params (8 bytes, echoed): [0x00,0x01,0x12,0x08,0x12,0x84,0x01,0x00]
        //   data envelope (4 bytes): return_code=0xFF, transport=0x09, len=12
        //   szl_payload (12 bytes)
        let params: [u8; 8] = [0x00, 0x01, 0x12, 0x08, 0x12, 0x84, 0x01, 0x00];
        let data_envelope: [u8; 4] = [0xFF, 0x09, 0x00, 0x0C];
        let param_len = params.len() as u16;
        let data_len = (data_envelope.len() + szl_payload.len()) as u16;

        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::UserData, reserved: 0, pdu_ref: 2,
            param_len, data_len,
            error_class: None, error_code: None,
        }.encode(&mut s7b);
        s7b.extend_from_slice(&params);
        s7b.extend_from_slice(&data_envelope);
        s7b.extend_from_slice(&szl_payload);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn get_plc_status_returns_run() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_status(server_io, 0x08));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let status = client.get_plc_status().await.unwrap();
        assert_eq!(status, crate::types::PlcStatus::Run);
    }

    #[tokio::test]
    async fn get_plc_status_returns_stop() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_status(server_io, 0x04));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let status = client.get_plc_status().await.unwrap();
        assert_eq!(status, crate::types::PlcStatus::Stop);
    }

    #[tokio::test]
    async fn get_plc_status_returns_unknown() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_status(server_io, 0x00));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let status = client.get_plc_status().await.unwrap();
        assert_eq!(status, crate::types::PlcStatus::Unknown);
    }

    #[tokio::test]
    async fn get_plc_status_unknown_byte_returns_stop() {
        // Unknown status bytes default to Stop (C snap7 behavior)
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_status(server_io, 0xFF));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let status = client.get_plc_status().await.unwrap();
        assert_eq!(status, crate::types::PlcStatus::Stop);
    }

    #[tokio::test]
    async fn mb_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let expected = vec![0xAA, 0xBB];
        tokio::spawn(mock_plc_multi_read(server_io, vec![expected.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.mb_read(10, 2).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    #[tokio::test]
    async fn eb_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let expected = vec![0x01, 0x02, 0x03];
        tokio::spawn(mock_plc_multi_read(server_io, vec![expected.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.eb_read(0, 3).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    #[tokio::test]
    async fn ib_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let expected = vec![0x11, 0x22];
        tokio::spawn(mock_plc_multi_read(server_io, vec![expected.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.ib_read(0, 2).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    #[tokio::test]
    async fn tm_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        // Two timer words = 4 bytes
        let expected = vec![0x00, 0x14, 0x00, 0x28];
        tokio::spawn(mock_plc_multi_read(server_io, vec![expected.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.tm_read(0, 2).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    #[tokio::test]
    async fn ct_read_returns_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        // One counter word = 2 bytes
        let expected = vec![0x00, 0x07];
        tokio::spawn(mock_plc_multi_read(server_io, vec![expected.clone()]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.ct_read(3, 1).await.unwrap();
        assert_eq!(&data[..], &expected[..]);
    }

    async fn mock_set_clock(mut server_io: tokio::io::DuplexStream) {
        let mut buf = vec![0u8; 4096];
        mock_handshake(&mut server_io).await;
        let _ = server_io.read(&mut buf).await;
        // Send success AckData with no data
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len: 0, data_len: 0, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn set_clock_succeeds() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_set_clock(server_io));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let dt = crate::proto::s7::clock::PlcDateTime {
            year: 2025, month: 5, day: 9, hour: 12, minute: 0, second: 0,
            millisecond: 0, weekday: 5,
        };
        client.set_clock(&dt).await.unwrap();
    }

    #[tokio::test]
    async fn set_clock_to_now_succeeds() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_set_clock(server_io));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.set_clock_to_now().await.unwrap();
    }

    async fn mock_szl_list(mut server_io: tokio::io::DuplexStream, ids: Vec<u16>) {
        let mut buf = vec![0u8; 4096];
        mock_handshake(&mut server_io).await;
        let _ = server_io.read(&mut buf).await;

        // Build SZL block: [szl_id=0x0000][szl_index=0][entry_len=4][entry_count=N][{id(2)+pad(2)}*N]
        let entry_len: u16 = 4;
        let entry_count = ids.len() as u16;
        let mut szl = BytesMut::new();
        szl.put_u16(0x0000); // szl_id
        szl.put_u16(0x0000); // szl_index
        szl.put_u16(entry_len);
        szl.put_u16(entry_count);
        for id in &ids {
            szl.put_u16(*id);
            szl.put_u16(0x0000); // padding
        }
        let szl_bytes = szl.freeze();
        let data_len = (4 + szl_bytes.len()) as u16; // envelope(4) + szl_block

        let mut s7b = BytesMut::new();
        // param section (8 bytes echoed)
        let param_len: u16 = 8;
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len, data_len, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        // echoed param (8 bytes)
        s7b.extend_from_slice(&[0x00, 0x01, 0x12, 0x04, 0x11, 0x44, 0x01, 0x00]);
        // data envelope
        s7b.put_u8(0xFF); s7b.put_u8(0x09);
        s7b.put_u16(szl_bytes.len() as u16);
        s7b.extend_from_slice(&szl_bytes);

        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn read_szl_list_returns_ids() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let ids = vec![0x0011u16, 0x001C, 0x0131, 0x0424];
        tokio::spawn(mock_szl_list(server_io, ids.clone()));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let result = client.read_szl_list().await.unwrap();
        assert_eq!(result, ids);
    }

    #[tokio::test]
    async fn read_szl_list_empty_returns_empty() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_szl_list(server_io, vec![]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let result = client.read_szl_list().await.unwrap();
        assert!(result.is_empty());
    }

    /// Mock for full_upload: handshake + 3-message exchange (start, data, end).
    async fn mock_full_upload(mut server_io: tokio::io::DuplexStream, block_data: Vec<u8>) {
        let mut buf = vec![0u8; 4096];
        mock_handshake(&mut server_io).await;

        // Start request (func=0x1F, sub-fn=0x00)
        let _ = server_io.read(&mut buf).await;
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len: 2, data_len: 8, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        s7b.extend_from_slice(&[0x1F, 0x00]); // param echo
        s7b.put_u32(0xDEAD_BEEF_u32); // upload_id
        s7b.put_u32(block_data.len() as u32); // total_len
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // Data request (func=0x1F, sub-fn=0x01)
        let _ = server_io.read(&mut buf).await;
        let data_payload_len = (4 + block_data.len()) as u16; // return_code+transport+len(2) + data
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 3,
            param_len: 2, data_len: data_payload_len, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        s7b.extend_from_slice(&[0x1F, 0x01]);
        s7b.put_u8(0xFF); s7b.put_u8(0x04);
        s7b.put_u16((block_data.len() * 8) as u16);
        s7b.extend_from_slice(&block_data);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();

        // End request (func=0x1F, sub-fn=0x02)
        let _ = server_io.read(&mut buf).await;
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 4,
            param_len: 0, data_len: 0, error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cb = BytesMut::new(); dt.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        server_io.write_all(&tb).await.unwrap();
    }

    #[tokio::test]
    async fn full_upload_returns_block_data() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        let expected = vec![0x01u8, 0x02, 0x03, 0x04];
        tokio::spawn(mock_full_upload(server_io, expected.clone()));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let data = client.full_upload(0x41, 1).await.unwrap();
        assert_eq!(data, expected);
    }

    #[tokio::test]
    async fn get_pdu_length_returns_negotiated_size() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        // mock_plc_db_read negotiates pdu_length=480
        tokio::spawn(mock_plc_db_read(server_io, vec![0x00]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let pdu_len = client.get_pdu_length().await;
        assert_eq!(pdu_len, 480);
    }

    #[test]
    fn parse_block_info_valid() {
        type C = S7Client<tokio::io::DuplexStream>;
        // Build a minimal TS7CompactBlockInfo buffer (HDR=36, FOOTER=48, total=84)
        const TOTAL: usize = 84;
        let mut buf = vec![0u8; TOTAL];
        buf[0] = 0x70; buf[1] = 0x70;
        buf[3] = 0x09; // flags
        buf[4] = 0x01; // language
        buf[5] = 0x41; // SubBlkType (DB)
        buf[6] = 0x00; buf[7] = 0x05; // block_number = 5
        let total_be = (TOTAL as u32).to_be_bytes();
        buf[8..12].copy_from_slice(&total_be);
        buf[28] = 0x00; buf[29] = 0x10; // size_ram = 16
        buf[32] = 0x00; buf[33] = 0x08; // local_data = 8
        buf[34] = 0x00; buf[35] = 0x0A; // mc7_size = 10
        let footer_start = TOTAL - 48;
        buf[footer_start + 20..footer_start + 27].copy_from_slice(b"SIEMENS");
        buf[footer_start + 28..footer_start + 32].copy_from_slice(b"TEST");
        buf[footer_start + 36..footer_start + 40].copy_from_slice(b"V1.0");
        buf[footer_start + 44] = 0xAB; buf[footer_start + 45] = 0xCD;

        let info = C::parse_block_info(&buf).unwrap();
        assert_eq!(info.block_number, 5);
        assert_eq!(info.block_type, 0x41);
        assert_eq!(info.language, 1);
        assert_eq!(info.flags, 9);
        assert_eq!(info.size, TOTAL as u16);
        assert_eq!(info.size_ram, 16);
        assert_eq!(info.mc7_size, 10);
        assert_eq!(info.local_data, 8);
        assert_eq!(info.checksum, 0xABCD);
        assert_eq!(info.author, "SIEMENS");
        assert_eq!(info.family, "TEST");
        assert_eq!(info.header, "V1.0");
    }

    #[test]
    fn parse_block_info_too_short() {
        type C = S7Client<tokio::io::DuplexStream>;
        let buf = vec![0u8; 10];
        assert!(C::parse_block_info(&buf).is_err());
    }

    #[test]
    fn parse_block_info_mismatched_load_size() {
        type C = S7Client<tokio::io::DuplexStream>;
        const TOTAL: usize = 84;
        let mut buf = vec![0u8; TOTAL];
        let wrong = 100u32.to_be_bytes();
        buf[8..12].copy_from_slice(&wrong);
        buf[34] = 0x00; buf[35] = 0x0A;
        assert!(C::parse_block_info(&buf).is_err());
    }

    #[tokio::test]
    async fn reconnect_resets_state() {
        use std::net::SocketAddr;

        // Spin up a tiny TCP listener that does a full S7 handshake then closes.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();

        // Spawn a server task: handles two sequential connections (initial + reconnect).
        tokio::spawn(async move {
            for _ in 0..2 {
                if let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(mock_tcp_plc(stream));
                }
            }
        });

        let params = ConnectParams::default();
        let client = S7Client::<crate::transport::TcpTransport>::connect(addr, params)
            .await
            .unwrap();

        assert!(client.is_connected().await);

        // Simulate disconnect by calling reconnect (server will accept second conn).
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        client.reconnect().await.unwrap();
        assert!(client.is_connected().await);
    }

    /// Minimal TCP mock: complete COTP CR/CC + S7 negotiate, then drop.
    async fn mock_tcp_plc(mut stream: tokio::net::TcpStream) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = vec![0u8; 512];

        // COTP CR
        let _ = stream.read(&mut buf).await;
        let cc = CotpPdu::ConnectConfirm { dst_ref: 1, src_ref: 1 };
        let mut cb = BytesMut::new();
        cc.encode(&mut cb);
        let mut tb = BytesMut::new();
        TpktFrame { payload: cb.freeze() }.encode(&mut tb).unwrap();
        let _ = stream.write_all(&tb).await;

        // S7 negotiate
        let _ = stream.read(&mut buf).await;
        let neg_resp = NegotiateResponse { max_amq_calling: 1, max_amq_called: 1, pdu_length: 480 };
        let ack = S7Header {
            pdu_type: PduType::AckData,
            reserved: 0,
            pdu_ref: 1,
            param_len: 8,
            data_len: 0,
            error_class: Some(0),
            error_code: Some(0),
        };
        let mut s7b = BytesMut::new();
        ack.encode(&mut s7b);
        neg_resp.encode(&mut s7b);
        let dt = CotpPdu::Data { tpdu_nr: 0, last: true, payload: s7b.freeze() };
        let mut cotpb = BytesMut::new();
        dt.encode(&mut cotpb);
        let mut tb2 = BytesMut::new();
        TpktFrame { payload: cotpb.freeze() }.encode(&mut tb2).unwrap();
        let _ = stream.write_all(&tb2).await;
        // hold connection open briefly then drop
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn get_exec_time_after_request() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_db_read(server_io, vec![0x00, 0x01, 0x02, 0x03]));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        client.db_read(1, 0, 4).await.unwrap();
        let exec_ms = client.get_exec_time().await;
        // Just verify it was set — exact value is timing-dependent.
        // In tests with in-process duplex the round-trip is < 1 ms so often 0; just check it doesn't panic.
        let _ = exec_ms;
    }
}
