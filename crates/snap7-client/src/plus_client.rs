use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::proto::s7commplus::frame::{S7PlusFrame, Version};
use crate::proto::s7commplus::multivar::{GetVarRequest, GetVarResponse, SetVarRequest};
use crate::proto::tpkt::TpktFrame;

use crate::error::Error;
use crate::plus_connection::{plus_connect, PlusConnection};
use crate::transport::TcpTransport;

/// Inner mutable state for an `S7PlusClient`.
pub(crate) struct PlusInner<T> {
    pub transport: T,
    pub conn: PlusConnection,
}

/// A client for S7CommPlus (S7-1200/1500 "integrity mode") communication.
pub struct S7PlusClient<T: AsyncRead + AsyncWrite + Unpin + Send> {
    pub(crate) inner: Mutex<PlusInner<T>>,
}

fn db_lid(db: u16, byte_offset: u32) -> (u32, u32) {
    let crc = 0x48u32;
    let lid = 0x8400_0000u32 | ((db as u32) << 12) | (byte_offset & 0xFFF);
    (crc, lid)
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> S7PlusClient<T> {
    /// Read `length` bytes from DB `db` at byte offset `start`.
    pub async fn db_read(&self, db: u16, start: u32, length: u16) -> Result<Bytes, Error> {
        let mut inner = self.inner.lock().await;
        let seqnum = inner.conn.seqnum;
        inner.conn.seqnum = seqnum.wrapping_add(1);
        let (crc, lid) = db_lid(db, start);

        let req = GetVarRequest {
            seqnum,
            session_id: inner.conn.session_id,
            crc,
            lid,
        };
        let mut da = BytesMut::new();
        req.encode(&mut da);

        let version = inner.conn.version.clone();
        send_plus(&mut inner.transport, version, da.freeze()).await?;
        let data = recv_plus_data(&mut inner.transport).await?;
        let mut b = data;
        let resp = GetVarResponse::decode(&mut b).map_err(Error::Proto)?;
        let len = length as usize;
        if resp.value.len() >= len {
            Ok(resp.value.slice(..len))
        } else {
            Ok(resp.value)
        }
    }

    /// Write `data` to DB `db` at byte offset `start`.
    pub async fn db_write(&self, db: u16, start: u32, data: &[u8]) -> Result<(), Error> {
        let mut inner = self.inner.lock().await;
        let seqnum = inner.conn.seqnum;
        inner.conn.seqnum = seqnum.wrapping_add(1);
        let (crc, lid) = db_lid(db, start);

        let req = SetVarRequest {
            seqnum,
            session_id: inner.conn.session_id,
            crc,
            lid,
            value: Bytes::copy_from_slice(data),
        };
        let mut da = BytesMut::new();
        req.encode(&mut da);

        let version = inner.conn.version.clone();
        send_plus(&mut inner.transport, version, da.freeze()).await?;
        let _resp = recv_plus_data(&mut inner.transport).await?;
        Ok(())
    }
}

impl S7PlusClient<TcpTransport> {
    /// Connect to a PLC at `addr` using the S7CommPlus CreateObject handshake.
    pub async fn connect(
        addr: std::net::SocketAddr,
        params: crate::types::ConnectParams,
    ) -> Result<Self, Error> {
        let transport = TcpTransport::connect(addr, params.connect_timeout).await?;
        let (conn, transport) = plus_connect(transport).await?;
        Ok(S7PlusClient {
            inner: Mutex::new(PlusInner { transport, conn }),
        })
    }
}

async fn send_plus<T>(transport: &mut T, version: Version, data: Bytes) -> Result<(), Error>
where
    T: AsyncWrite + Unpin,
{
    let frame = S7PlusFrame { version, data };
    let mut fb = BytesMut::new();
    frame.encode(&mut fb).map_err(Error::Proto)?;
    let tpkt = TpktFrame {
        payload: fb.freeze(),
    };
    let mut tb = BytesMut::new();
    tpkt.encode(&mut tb).map_err(Error::Proto)?;
    transport.write_all(&tb).await?;
    Ok(())
}

async fn recv_plus_data<T>(transport: &mut T) -> Result<Bytes, Error>
where
    T: AsyncRead + Unpin,
{
    let mut hdr = [0u8; 4];
    transport.read_exact(&mut hdr).await?;
    let total = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
    let mut payload = vec![0u8; total.saturating_sub(4)];
    transport.read_exact(&mut payload).await?;
    let mut b = Bytes::from(payload);
    let frame = S7PlusFrame::decode(&mut b).map_err(Error::Proto)?;
    Ok(frame.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use tokio::io::AsyncWriteExt;

    fn build_get_var_response(session_id: u32, seqnum: u16, value: &[u8]) -> Vec<u8> {
        use bytes::BytesMut;
        use crate::proto::s7commplus::frame::{S7PlusFrame, Version};
        use crate::proto::s7commplus::session::OPCODE_RESPONSE;
        use crate::proto::tpkt::TpktFrame;

        let mut da = BytesMut::new();
        da.put_u8(OPCODE_RESPONSE);
        da.put_u16(0x0000);
        da.put_u16(0x054C); // FC_GET_MULTI_VAR
        da.put_u16(0x0000);
        da.put_u16(seqnum);
        da.put_u32(session_id);
        da.put_u8(0x00);
        // payload: return_code(1) + len(2 BE) + value
        da.put_u8(0x0A);
        da.put_u16(value.len() as u16);
        da.put_slice(value);

        let frame = S7PlusFrame {
            version: Version::V1,
            data: da.freeze(),
        };
        let mut fb = BytesMut::new();
        frame.encode(&mut fb).unwrap();
        let tpkt = TpktFrame {
            payload: fb.freeze(),
        };
        let mut tb = BytesMut::new();
        tpkt.encode(&mut tb).unwrap();
        tb.to_vec()
    }

    #[tokio::test]
    async fn plus_db_read_returns_value() {
        let session_id = 0x0000_0001_u32;
        let value = [0x3F, 0x80, 0x00, 0x00]; // 1.0f32 BE
        let response = build_get_var_response(session_id, 2, &value);

        let (mut server, client_io) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut server, &mut buf).await;
            server.write_all(&response).await.unwrap();
        });

        let conn = PlusConnection {
            session_id,
            seqnum: 2,
            version: crate::proto::s7commplus::frame::Version::V1,
        };
        let client = S7PlusClient {
            inner: tokio::sync::Mutex::new(PlusInner {
                transport: client_io,
                conn,
            }),
        };
        let data = client.db_read(1, 0, 4).await.unwrap();
        assert_eq!(&data[..], &[0x3F, 0x80, 0x00, 0x00]);
    }
}
