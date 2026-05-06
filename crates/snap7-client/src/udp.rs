use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;

pub struct UdpTransport {
    socket: UdpSocket,
    // overflow buffer: excess datagram bytes that didn't fit the caller's ReadBuf
    read_buf: BytesMut,
}

// UdpSocket and BytesMut are both Unpin; required for S7Transport: Unpin bound
impl Unpin for UdpTransport {}

impl UdpTransport {
    pub async fn connect(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(addr).await?;
        Ok(UdpTransport {
            socket,
            read_buf: BytesMut::with_capacity(65535),
        })
    }
}

impl AsyncRead for UdpTransport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // Drain buffered data first
        if !this.read_buf.is_empty() {
            let n = this.read_buf.len().min(buf.remaining());
            buf.put_slice(&this.read_buf[..n]);
            let _ = this.read_buf.split_to(n);
            return Poll::Ready(Ok(()));
        }

        // Reuse read_buf as the receive scratch area (it is empty at this point)
        this.read_buf.resize(65535, 0);
        let mut rb = ReadBuf::new(&mut this.read_buf);
        match Pin::new(&this.socket).poll_recv(cx, &mut rb) {
            Poll::Ready(Ok(())) => {
                let filled = rb.filled().len();
                let n = filled.min(buf.remaining());
                buf.put_slice(&this.read_buf[..n]);
                // Keep overflow bytes in read_buf for the next poll_read
                let _ = this.read_buf.split_to(n);
                this.read_buf.truncate(filled - n);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => {
                this.read_buf.clear();
                Poll::Ready(Err(e))
            }
            Poll::Pending => {
                this.read_buf.clear();
                Poll::Pending
            }
        }
    }
}

impl AsyncWrite for UdpTransport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&self.get_mut().socket).poll_send(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn udp_transport_loopback_roundtrip() {
        let server = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server.local_addr().unwrap();

        let mut client = UdpTransport::connect(server_addr).await.unwrap();

        // Send from client to server
        client.write_all(b"hello").await.unwrap();

        // Server receives it
        let mut buf = [0u8; 64];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello");

        // Server replies
        server.send_to(b"world", peer).await.unwrap();

        // Client reads reply
        let mut rbuf = [0u8; 64];
        let n = client.read(&mut rbuf).await.unwrap();
        assert_eq!(&rbuf[..n], b"world");
    }
}
