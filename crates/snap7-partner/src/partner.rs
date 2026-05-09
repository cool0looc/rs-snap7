use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::{
    error::{Error, Result},
    proto::{
        BSendDataHdr, BSendParams, GR_BSEND, GR_BSEND_ACK,
        BSEND_PARAMS_LEN, BSEND_DATA_HDR_LEN, S7_HDR_LEN,
    },
    transport::{active_handshake, passive_handshake, recv_iso_frame, send_iso_frame},
};

const MAX_BSEND_SIZE: usize = 65536;

/// Connection status of a partner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartnerStatus {
    Disconnected,
    Connected,
}

struct Inner {
    stream: TcpStream,
    pdu_size: u16,
}

/// Async S7 Partner — supports both active (client-style) and passive (server-style) roles.
///
/// - **Active** partner connects to a remote IP and acts as the initiator.
/// - **Passive** partner listens on a local port and waits for the remote to connect.
///
/// Once connected both peers can `bsend` and `brecv` in any order.
pub struct S7Partner {
    inner: Mutex<Option<Inner>>,
    seq: AtomicU8,
    pdu_ref: AtomicU16,
    connected: AtomicBool,
    bytes_sent: AtomicU64,
    bytes_recv: AtomicU64,
    send_errors: AtomicU32,
    recv_errors: AtomicU32,
    pub recv_timeout: Duration,
    pub send_timeout: Duration,
}

impl S7Partner {
    fn new_unconnected() -> Self {
        Self {
            inner: Mutex::new(None),
            seq: AtomicU8::new(0),
            pdu_ref: AtomicU16::new(1),
            connected: AtomicBool::new(false),
            bytes_sent: AtomicU64::new(0),
            bytes_recv: AtomicU64::new(0),
            send_errors: AtomicU32::new(0),
            recv_errors: AtomicU32::new(0),
            recv_timeout: Duration::from_secs(3),
            send_timeout: Duration::from_secs(3),
        }
    }

    /// Returns whether the partner is currently connected.
    pub fn get_status(&self) -> PartnerStatus {
        if self.connected.load(Ordering::Relaxed) {
            PartnerStatus::Connected
        } else {
            PartnerStatus::Disconnected
        }
    }

    /// Returns `(bytes_sent, bytes_received)`.
    pub fn get_stats(&self) -> (u64, u64) {
        (
            self.bytes_sent.load(Ordering::Relaxed),
            self.bytes_recv.load(Ordering::Relaxed),
        )
    }

    /// Returns `(send_errors, recv_errors)`.
    pub fn get_error_counts(&self) -> (u32, u32) {
        (
            self.send_errors.load(Ordering::Relaxed),
            self.recv_errors.load(Ordering::Relaxed),
        )
    }

    fn next_seq(&self) -> u8 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    fn next_pdu_ref(&self) -> u16 {
        self.pdu_ref.fetch_add(1, Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Connect helpers
    // -----------------------------------------------------------------------

    /// Active partner: connect to `addr`, perform handshake.
    pub async fn connect(addr: SocketAddr) -> Result<Arc<Self>> {
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        let this = Arc::new(Self::new_unconnected());
        let (pdu_size, stream) = drive_active_handshake(stream).await?;
        *this.inner.lock().await = Some(Inner { stream, pdu_size });
        this.connected.store(true, Ordering::Relaxed);
        Ok(this)
    }

    /// Passive partner: bind `addr`, accept one connection, perform handshake.
    pub async fn listen(addr: SocketAddr) -> Result<Arc<Self>> {
        let listener = TcpListener::bind(addr).await?;
        let (stream, _) = listener.accept().await?;
        stream.set_nodelay(true)?;
        let (pdu_size, stream) = drive_passive_handshake(stream).await?;
        let this = Arc::new(Self::new_unconnected());
        *this.inner.lock().await = Some(Inner { stream, pdu_size });
        this.connected.store(true, Ordering::Relaxed);
        Ok(this)
    }

    /// Bind and return the listener separately so callers can retrieve the bound addr.
    pub async fn bind(addr: SocketAddr) -> Result<(TcpListener, Arc<Self>)> {
        let listener = TcpListener::bind(addr).await?;
        let partner = Arc::new(Self::new_unconnected());
        Ok((listener, partner))
    }

    /// Accept one connection on an already-bound listener, finish handshake.
    pub async fn accept(partner: &Arc<Self>, listener: &TcpListener) -> Result<()> {
        let (stream, _) = listener.accept().await?;
        stream.set_nodelay(true)?;
        let (pdu_size, stream) = drive_passive_handshake(stream).await?;
        *partner.inner.lock().await = Some(Inner { stream, pdu_size });
        partner.connected.store(true, Ordering::Relaxed);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // BSend
    // -----------------------------------------------------------------------

    /// Send `data` to the partner with the given `r_id`. Fragments automatically
    /// if `data.len()` exceeds one PDU slice.
    pub async fn bsend(&self, r_id: u32, data: &[u8]) -> Result<()> {
        if data.len() > MAX_BSEND_SIZE {
            return Err(Error::InvalidPdu("data exceeds 64 kB BSend limit"));
        }
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(Error::NotConnected)?;

        // Max payload per slice: pdu_size - S7_HDR(10) - BSendParams(12) - DataHdr(12) - 2
        let max_slice = (inner.pdu_size as usize)
            .saturating_sub(S7_HDR_LEN + BSEND_PARAMS_LEN + BSEND_DATA_HDR_LEN + 2)
            .max(1);

        let pdu_ref = self.next_pdu_ref();
        let id_seq = if data.len() > max_slice { self.next_seq() } else { 0x00 };
        let mut seq_out: u8 = 0;
        let mut offset = 0;
        let mut first = true;

        while offset < data.len() {
            let end = (offset + max_slice).min(data.len());
            let slice = &data[offset..end];
            let is_last = end == data.len();
            let eos = if is_last { 0x00 } else { 0x01 };

            let extra = if first { 2usize } else { 0 };
            let data_len = (BSEND_DATA_HDR_LEN + slice.len() + extra) as u16;
            let param_len = BSEND_PARAMS_LEN as u16;

            // Build frame
            let mut frame = Vec::with_capacity(S7_HDR_LEN + param_len as usize + data_len as usize);
            encode_s7_userdata_header(&mut frame, pdu_ref, param_len, data_len);
            BSendParams {
                tg: GR_BSEND,
                sub_fun: 0x01,
                seq: seq_out,
                id_seq,
                eos,
                err: 0,
            }
            .encode(&mut frame);

            let dh_len = (slice.len() + 8 + extra) as u16;
            BSendDataHdr { len: dh_len, r_id }.encode(&mut frame);
            if first {
                let total_len = data.len() as u16;
                frame.push((total_len >> 8) as u8);
                frame.push(total_len as u8);
            }
            frame.extend_from_slice(slice);

            if let Err(e) = send_iso_frame(&mut inner.stream, &frame).await {
                self.send_errors.fetch_add(1, Ordering::Relaxed);
                return Err(e.into());
            }
            self.bytes_sent.fetch_add(frame.len() as u64, Ordering::Relaxed);

            // Read acknowledgement
            let resp = match recv_iso_frame(&mut inner.stream).await {
                Ok(r) => r,
                Err(e) => {
                    self.recv_errors.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
            };
            let rparams = parse_s7_bsend_params(&resp)?;
            if rparams.err != 0 {
                self.send_errors.fetch_add(1, Ordering::Relaxed);
                return Err(Error::SendRefused(rparams.err));
            }
            seq_out = rparams.seq;

            offset = end;
            first = false;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // BRecv
    // -----------------------------------------------------------------------

    /// Wait for the partner to send a BSend frame. Returns `(r_id, data)`.
    pub async fn brecv(&self) -> Result<(u32, Vec<u8>)> {
        let mut guard = self.inner.lock().await;
        let inner = guard.as_mut().ok_or(Error::NotConnected)?;

        let mut buf: Vec<u8> = Vec::new();
        let mut r_id;
        let mut seq_out = self.next_seq();
        let mut first = true;

        loop {
            let frame = match recv_iso_frame(&mut inner.stream).await {
                Ok(f) => f,
                Err(e) => {
                    self.recv_errors.fetch_add(1, Ordering::Relaxed);
                    return Err(e);
                }
            };
            self.bytes_recv.fetch_add(frame.len() as u64, Ordering::Relaxed);

            let params = parse_s7_bsend_params(&frame)?;
            if params.tg != GR_BSEND {
                return Err(Error::InvalidPdu("expected BSend PDU"));
            }

            // Data starts after S7_HDR + BSendParams
            let data_offset = S7_HDR_LEN + BSEND_PARAMS_LEN;
            if frame.len() < data_offset + BSEND_DATA_HDR_LEN {
                return Err(Error::InvalidPdu("frame too short for BSend data"));
            }
            let dh = BSendDataHdr::decode(&frame[data_offset..])
                .ok_or(Error::InvalidPdu("bad BSend data header"))?;
            r_id = dh.r_id;

            let extra = if first { 2usize } else { 0 };
            let payload_start = data_offset + BSEND_DATA_HDR_LEN + extra;
            let slice_len = (dh.len as usize).saturating_sub(8 + extra);
            if frame.len() < payload_start + slice_len {
                return Err(Error::InvalidPdu("BSend frame data truncated"));
            }
            buf.extend_from_slice(&frame[payload_start..payload_start + slice_len]);
            first = false;

            // Send acknowledgement
            let ack_param_len = BSEND_PARAMS_LEN as u16;
            let ack_data_len = 4u16;
            let mut ack = Vec::with_capacity(S7_HDR_LEN + ack_param_len as usize + ack_data_len as usize);
            encode_s7_userdata_header(&mut ack, self.next_pdu_ref(), ack_param_len, ack_data_len);
            BSendParams {
                tg: GR_BSEND_ACK,
                sub_fun: 0x01,
                seq: seq_out,
                id_seq: 0x00,
                eos: 0x00,
                err: 0,
            }
            .encode(&mut ack);
            // TBSendResData: 0x0A 0x00 0x00 0x00
            ack.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00]);
            if let Err(e) = send_iso_frame(&mut inner.stream, &ack).await {
                self.send_errors.fetch_add(1, Ordering::Relaxed);
                return Err(e.into());
            }
            self.bytes_sent.fetch_add(ack.len() as u64, Ordering::Relaxed);

            seq_out = seq_out.wrapping_add(1);

            if params.eos == 0x00 {
                break;
            }
        }

        Ok((r_id, buf))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (non-async)
// ---------------------------------------------------------------------------

fn encode_s7_userdata_header(buf: &mut Vec<u8>, pdu_ref: u16, param_len: u16, data_len: u16) {
    buf.push(0x32);
    buf.push(0x07); // PDU_TYPE_USERDATA
    buf.push(0x00); buf.push(0x00);
    buf.push((pdu_ref >> 8) as u8);
    buf.push(pdu_ref as u8);
    buf.push((param_len >> 8) as u8);
    buf.push(param_len as u8);
    buf.push((data_len >> 8) as u8);
    buf.push(data_len as u8);
}

fn parse_s7_bsend_params(frame: &[u8]) -> Result<BSendParams> {
    if frame.len() < S7_HDR_LEN + BSEND_PARAMS_LEN {
        return Err(Error::InvalidPdu("frame too short for BSend params"));
    }
    BSendParams::decode(&frame[S7_HDR_LEN..])
        .ok_or(Error::InvalidPdu("malformed BSend params"))
}

// ---------------------------------------------------------------------------
// Handshake helpers that own the TcpStream
// ---------------------------------------------------------------------------

async fn drive_active_handshake(mut stream: TcpStream) -> Result<(u16, TcpStream)> {
    let pdu = active_handshake(&mut stream).await?;
    Ok((pdu, stream))
}

async fn drive_passive_handshake(mut stream: TcpStream) -> Result<(u16, TcpStream)> {
    let pdu = passive_handshake(&mut stream).await?;
    Ok((pdu, stream))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn any_addr() -> SocketAddr {
        "127.0.0.1:0".parse().unwrap()
    }

    #[tokio::test]
    async fn bsend_brecv_roundtrip() {
        let (listener, passive) = S7Partner::bind(any_addr()).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let passive_clone = Arc::clone(&passive);
        let server_task = tokio::spawn(async move {
            S7Partner::accept(&passive_clone, &listener).await.unwrap();
            passive_clone.brecv().await.unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let active = S7Partner::connect(addr).await.unwrap();
        let payload = b"hello partner";
        active.bsend(0xDEADBEEF, payload).await.unwrap();

        let (r_id, data) = server_task.await.unwrap();
        assert_eq!(r_id, 0xDEADBEEF);
        assert_eq!(data, payload);
    }

    #[tokio::test]
    async fn bsend_brecv_large_payload() {
        let data: Vec<u8> = (0u8..=255).cycle().take(4000).collect();

        let (listener, passive) = S7Partner::bind(any_addr()).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let passive_clone = Arc::clone(&passive);
        let expected = data.clone();
        let server_task = tokio::spawn(async move {
            S7Partner::accept(&passive_clone, &listener).await.unwrap();
            passive_clone.brecv().await.unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let active = S7Partner::connect(addr).await.unwrap();
        active.bsend(0x0000_0001, &data).await.unwrap();

        let (r_id, received) = server_task.await.unwrap();
        assert_eq!(r_id, 0x0000_0001);
        assert_eq!(received, expected);
    }

    #[tokio::test]
    async fn bidirectional_exchange() {
        let (listener, passive) = S7Partner::bind(any_addr()).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let passive_clone = Arc::clone(&passive);
        let server_task = tokio::spawn(async move {
            S7Partner::accept(&passive_clone, &listener).await.unwrap();
            // Passive receives, then sends back
            let (r_id, data) = passive_clone.brecv().await.unwrap();
            passive_clone.bsend(r_id + 1, &data).await.unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let active = S7Partner::connect(addr).await.unwrap();
        active.bsend(0x42, b"ping").await.unwrap();
        let (r_id, data) = active.brecv().await.unwrap();
        assert_eq!(r_id, 0x43);
        assert_eq!(data, b"ping");

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn stats_and_status() {
        use crate::PartnerStatus;

        let (listener, passive) = S7Partner::bind(any_addr()).await.unwrap();
        let addr = listener.local_addr().unwrap();

        assert_eq!(passive.get_status(), PartnerStatus::Disconnected);
        let (sent_before, recv_before) = passive.get_stats();
        assert_eq!(sent_before, 0);
        assert_eq!(recv_before, 0);

        let passive_clone = Arc::clone(&passive);
        let server_task = tokio::spawn(async move {
            S7Partner::accept(&passive_clone, &listener).await.unwrap();
            assert_eq!(passive_clone.get_status(), PartnerStatus::Connected);
            passive_clone.brecv().await.unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let active = S7Partner::connect(addr).await.unwrap();
        assert_eq!(active.get_status(), PartnerStatus::Connected);
        active.bsend(0x01, b"stats_test").await.unwrap();

        server_task.await.unwrap();

        let (sent, _recv) = active.get_stats();
        assert!(sent > 0, "active should have sent bytes");
    }
}
