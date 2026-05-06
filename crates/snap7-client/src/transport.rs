use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::error::{Error, Result};

pub trait S7Transport: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> S7Transport for T {}

pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    pub async fn connect(addr: SocketAddr, timeout: Duration) -> Result<Self> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Timeout(timeout))?
            .map_err(|_| Error::ConnectionRefused)?;
        stream.set_nodelay(true)?;
        Ok(TcpTransport { stream })
    }
}

impl AsyncRead for TcpTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for TcpTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn duplex_transport_write_read() {
        let (mut client_half, mut server_half) = duplex(1024);
        server_half.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        client_half.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");
    }
}
