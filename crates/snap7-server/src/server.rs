use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::{
    dispatch::dispatch_loop, error::Result, handshake::server_handshake, store::DataStore,
};

/// Configuration for [`S7Server`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub max_connections: usize,
}

/// TCP listener that accepts connections and runs the full S7 pipeline per connection.
pub struct S7Server {
    listener: TcpListener,
    semaphore: Arc<Semaphore>,
}

impl S7Server {
    /// Bind a TCP listener at `config.bind_addr`.
    pub async fn bind(config: ServerConfig) -> Result<Self> {
        let listener = TcpListener::bind(config.bind_addr).await?;
        let semaphore = Arc::new(Semaphore::new(config.max_connections));
        Ok(Self {
            listener,
            semaphore,
        })
    }

    /// Return the local address the server is listening on.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept connections and serve them against `store` until an accept error occurs.
    pub async fn serve(self, store: DataStore) -> Result<()> {
        loop {
            // Acquire a permit — blocks when at max_connections.
            let permit = Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .expect("semaphore closed");

            let (stream, _peer) = self.listener.accept().await?;
            let store = store.clone();

            tokio::spawn(async move {
                let _permit = permit; // keep permit alive for connection lifetime
                if let Err(e) = serve_one(stream, store).await {
                    // Log connection errors at debug level; they are expected
                    // (e.g., client disconnecting mid-handshake).
                    let _ = e; // suppress unused-variable warning in release builds
                }
            });
        }
    }
}

/// Handle a single accepted connection: set TCP_NODELAY, run handshake, then dispatch loop.
async fn serve_one(mut stream: TcpStream, store: DataStore) -> Result<()> {
    stream.set_nodelay(true)?;
    let pdu_size = server_handshake(&mut stream).await?;
    dispatch_loop(&mut stream, pdu_size, store).await
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use snap7_client::{types::ConnectParams, S7Client};

    use super::*;
    use crate::store::DataStore;

    fn make_config() -> ServerConfig {
        ServerConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            max_connections: 4,
        }
    }

    #[tokio::test]
    async fn server_accepts_s7client_connection() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0x12, 0x34]);

        let server = S7Server::bind(make_config()).await.unwrap();
        let addr = server.local_addr().unwrap();

        tokio::spawn(server.serve(store));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let params = ConnectParams {
            rack: 0,
            slot: 1,
            ..ConnectParams::default()
        };
        let client = S7Client::connect(addr, params).await.unwrap();
        let data = client.db_read(1, 0, 2).await.unwrap();
        assert_eq!(&data[..], &[0x12, 0x34]);
    }

    #[tokio::test]
    async fn server_write_then_read() {
        let store = DataStore::new();

        let server = S7Server::bind(make_config()).await.unwrap();
        let addr = server.local_addr().unwrap();

        tokio::spawn(server.serve(store));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let params = ConnectParams::default();
        let client = S7Client::connect(addr, params).await.unwrap();

        client.db_write(2, 10, &[0xAB, 0xCD]).await.unwrap();
        let data = client.db_read(2, 10, 2).await.unwrap();
        assert_eq!(&data[..], &[0xAB, 0xCD]);
    }
}
