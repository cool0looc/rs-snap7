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
}

pub struct S7Client<T: AsyncRead + AsyncWrite + Unpin + Send> {
    inner: Mutex<Inner<T>>,
    params: ConnectParams,
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
            }),
            params,
        })
    }

    /// Return the current request timeout.
    pub fn request_timeout(&self) -> std::time::Duration {
        self.params.request_timeout
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
        inner.transport.write_all(&tb).await?;
        Ok(())
    }

    async fn recv_s7(inner: &mut Inner<T>) -> Result<(S7Header, Bytes)> {
        let timeout = inner.request_timeout;
        let mut tpkt_hdr = [0u8; 4];
        tokio::time::timeout(timeout, inner.transport.read_exact(&mut tpkt_hdr))
            .await
            .map_err(|_| Error::Timeout(timeout))??;
        let total = u16::from_be_bytes([tpkt_hdr[2], tpkt_hdr[3]]) as usize;
        if total < 4 {
            return Err(Error::UnexpectedResponse);
        }
        let mut payload = vec![0u8; total - 4];
        tokio::time::timeout(timeout, inner.transport.read_exact(&mut payload))
            .await
            .map_err(|_| Error::Timeout(timeout))??;
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

    /// Read the current PLC status (S7 function code 0x31).
    ///
    /// Returns one of [`PlcStatus::Run`], [`PlcStatus::Stop`], or
    /// [`PlcStatus::Unknown`].
    pub async fn get_plc_status(&self) -> Result<crate::types::PlcStatus> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);
        let param = Bytes::copy_from_slice(&[0x31, 0x00]);
        Self::send_s7(&mut inner, param, Bytes::new(), pdu_ref, PduType::Job).await?;
        let (header, mut body) = Self::recv_s7(&mut inner).await?;
        check_plc_error(&header, "get_plc_status")?;
        // Skip param echo: func (1) + reserved (1)
        if body.remaining() >= 2 {
            body.advance(2);
        }
        if body.remaining() < 1 {
            return Err(Error::UnexpectedResponse);
        }
        let status_byte = body.get_u8();
        match status_byte {
            0x00 => Ok(crate::types::PlcStatus::Unknown),
            0x04 => Ok(crate::types::PlcStatus::Stop),
            0x08 => Ok(crate::types::PlcStatus::Run),
            other => Err(Error::PlcError {
                code: other as u32,
                message: format!("unknown PLC status byte: 0x{other:02X}"),
            }),
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
            return Err(Error::UnexpectedResponse);
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

    // -- Block list & block info (via SZL + UserData) -------------------------

    /// List all blocks in the PLC grouped by type (SZL 0x0130).
    ///
    /// Returns a [`BlockList`] with the total block count and per-type entries.
    pub async fn list_blocks(&self) -> Result<crate::types::BlockList> {
        let payload = self.read_szl_payload(0x0130, 0x0000).await?;
        if payload.len() < 10 {
            return Err(Error::UnexpectedResponse);
        }
        let mut b = payload;
        let _block_len = b.get_u16();
        let _resp_szl_id = b.get_u16();
        let _szl_ix = b.get_u16();

        // S7-1500 format: strip sub-block header and entry prefixes.
        let mut szl_data = b;
        if szl_data.len() >= 2 && szl_data[0] == 0x04
            && (szl_data[1] == 0x02 || szl_data[1] == 0x03)
        {
            szl_data.advance(2);
            while szl_data.len() >= 4
                && szl_data[0] == 0xFF && szl_data[1] == 0x04
            {
                let entry_len = u16::from_be_bytes([szl_data[2], szl_data[3]]) as usize;
                let skip = 4 + entry_len;
                if skip > szl_data.len() { break; }
                szl_data.advance(skip);
            }
        }
        // Skip the optional SZL entry_length prefix (2 bytes).
        skip_szl_entry_header(&mut szl_data);
        let total_count = szl_data.get_u32();
        let mut entries = Vec::new();
        while szl_data.remaining() >= 4 {
            entries.push(crate::types::BlockListEntry {
                block_type: szl_data.get_u16(),
                count: szl_data.get_u16(),
            });
        }
        Ok(crate::types::BlockList {
            total_count,
            entries,
        })
    }

    /// Internal: send a UserData block-info request and return the raw response
    /// data section payload (4-byte envelope skipped).
    async fn block_info_query(
        &self,
        func: u8,
        block_type: u8,
        block_number: u16,
    ) -> Result<Bytes> {
        let mut inner = self.inner.lock().await;
        let pdu_ref = Self::next_pdu_ref(&mut inner);

        // UserData param for block info (function 0x13 or 0x14):
        //   [8-byte header] [block_type(1)] [0x00] [block_number(2)]
        let mut param_buf = BytesMut::with_capacity(12);
        param_buf.extend_from_slice(&[
            0x00, 0x01, 0x12, 0x04, func, 0x44, 0x01, 0x00,
            block_type, 0x00,
        ]);
        param_buf.put_u16(block_number);

        Self::send_s7(
            &mut inner,
            param_buf.freeze(),
            Bytes::new(),
            pdu_ref,
            PduType::UserData,
        )
        .await?;

        let (header, mut body) = Self::recv_s7(&mut inner).await?;

        // Skip echoed param section
        if body.remaining() < header.param_len as usize {
            return Err(Error::UnexpectedResponse);
        }
        body.advance(header.param_len as usize);

        // Skip 4-byte data envelope (return_code, transport, data_len)
        if body.remaining() < 4 {
            return Err(Error::UnexpectedResponse);
        }
        body.advance(4);

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
        // Minimum for a valid block info: 6-byte header + block_type + block_number + language + flags + ...
        if payload.len() < 24 {
            return Err(Error::UnexpectedResponse);
        }
        let mut b = payload;

        // Parse block info response (field order derived from S7 protocol):
        let _blk_type_hi = b.get_u16(); // may echo block type as u16
        let blk_number = b.get_u16();
        let language = b.get_u16();
        let flags = b.get_u16();
        let mc7_size = b.get_u16();
        let _size_lo = b.get_u16(); // load-memory size low word
        let size_ram = b.get_u16();
        let _size_ro = b.get_u16(); // 0 or RO-size
        let local_data = b.get_u16();
        let checksum = b.get_u16();
        let version = b.get_u16();

        // String fields: author(8), family(8), header(20?), date(8)
        let author = if b.remaining() >= 8 {
            String::from_utf8_lossy(&b[..8]).trim_end_matches('\0').trim().to_string()
        } else { String::new() };
        b.advance(8.min(b.remaining()));

        let family = if b.remaining() >= 8 {
            String::from_utf8_lossy(&b[..8]).trim_end_matches('\0').trim().to_string()
        } else { String::new() };
        b.advance(8.min(b.remaining()));

        let header = if b.remaining() >= 20 {
            String::from_utf8_lossy(&b[..20]).trim_end_matches('\0').trim().to_string()
        } else { String::new() };
        b.advance(20.min(b.remaining()));

        let date = if b.remaining() >= 8 {
            String::from_utf8_lossy(&b[..8]).trim_end_matches('\0').trim().to_string()
        } else { String::new() };

        // Reconstruct total size from the two size halves
        let size = ((_blk_type_hi as u32) << 16) | (b.len() as u32 & 0xFFFF);
        let size_u16 = size.min(0xFFFF) as u16;

        Ok(crate::types::BlockInfo {
            block_type: _blk_type_hi,
            block_number: blk_number,
            language,
            flags,
            size: size_u16,
            size_ram,
            mc7_size,
            local_data,
            checksum,
            version,
            author,
            family,
            header,
            date,
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
        Self::from_transport(transport, params).await
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

        // GetPlcStatus request — consume
        let _ = server_io.read(&mut buf).await;

        // Response: param echo [0x31, 0x00] + status byte
        let data = &[0x31u8, 0x00, status_byte]; // param(2) + data(1)
        let data_len = data.len() as u16;
        let mut s7b = BytesMut::new();
        S7Header {
            pdu_type: PduType::AckData, reserved: 0, pdu_ref: 2,
            param_len: 2, data_len,
            error_class: Some(0), error_code: Some(0),
        }.encode(&mut s7b);
        s7b.extend_from_slice(data);
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
    async fn get_plc_status_unknown_byte_returns_error() {
        let (client_io, server_io) = duplex(4096);
        let params = ConnectParams::default();
        tokio::spawn(mock_plc_status(server_io, 0xFF));
        let client = S7Client::from_transport(client_io, params).await.unwrap();
        let result = client.get_plc_status().await;
        assert!(result.is_err());
    }
}
