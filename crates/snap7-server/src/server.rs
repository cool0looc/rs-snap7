use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::{
    dispatch::dispatch_loop,
    error::Result,
    handshake::server_handshake,
    store::{CpuState, DataStore, EventInfo, ServerStatus},
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
        Ok(Self { listener, semaphore })
    }

    /// Bind to a specific address string (e.g. `"0.0.0.0:102"`).
    pub async fn start_to(addr: &str, max_connections: usize) -> Result<Self> {
        let bind_addr: SocketAddr = addr.parse().map_err(|e| {
            crate::error::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
        })?;
        Self::bind(ServerConfig { bind_addr, max_connections }).await
    }

    /// Return the local address the server is listening on.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept connections and serve them against `store` until an accept error occurs.
    pub async fn serve(self, store: DataStore) -> Result<()> {
        loop {
            let permit = Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .expect("semaphore closed");

            let (stream, _peer) = self.listener.accept().await?;
            let store = store.clone();

            tokio::spawn(async move {
                let _permit = permit;
                store.client_connected();
                let _ = serve_one(stream, store.clone()).await;
                store.client_disconnected();
            });
        }
    }

    // -- Delegated store management API (mirrors C snap7 Srv_* functions) ----

    /// Return server/CPU status and connected client count.
    pub fn get_status(store: &DataStore) -> ServerStatus {
        store.get_status()
    }

    /// Set the simulated CPU state.
    pub fn set_cpu_status(store: &DataStore, state: CpuState) {
        store.set_cpu_state(state);
    }

    /// Lock an area: writes to this area are silently ignored until unlocked.
    pub fn lock_area(store: &DataStore, area_code: u8) {
        store.lock_area(area_code);
    }

    /// Unlock a previously locked area.
    pub fn unlock_area(store: &DataStore, area_code: u8) {
        store.unlock_area(area_code);
    }

    /// Drain the event queue.
    pub fn clear_events(store: &DataStore) {
        store.clear_events();
    }

    /// Pop the oldest event from the queue. Returns `None` when empty.
    pub fn pick_event(store: &DataStore) -> Option<EventInfo> {
        store.pick_event()
    }

    /// Get the current event filter mask.
    pub fn get_mask(store: &DataStore) -> u32 {
        store.get_mask()
    }

    /// Set the event filter mask.
    pub fn set_mask(store: &DataStore, mask: u32) {
        store.set_mask(mask);
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

    #[test]
    fn get_status_reflects_cpu_state() {
        let store = DataStore::new();
        let status = S7Server::get_status(&store);
        assert_eq!(status.cpu_state, crate::store::CpuState::Stop);
        assert_eq!(status.clients_count, 0);
        assert!(status.server_running);
    }

    #[test]
    fn lock_area_blocks_writes() {
        let store = DataStore::new();
        store.write_bytes(1, 0, &[0xAA]);
        S7Server::lock_area(&store, crate::store::area::DATA_BLOCK);
        store.write_bytes(1, 0, &[0xFF]); // should be silently ignored
        let data = store.read_bytes(1, 0, 1);
        assert_eq!(data, vec![0xAA]);
        S7Server::unlock_area(&store, crate::store::area::DATA_BLOCK);
        store.write_bytes(1, 0, &[0xFF]);
        let data = store.read_bytes(1, 0, 1);
        assert_eq!(data, vec![0xFF]);
    }

    #[test]
    fn event_mask_and_queue() {
        let store = DataStore::new();
        S7Server::set_mask(&store, 0xFFFF_FFFF);
        assert_eq!(S7Server::get_mask(&store), 0xFFFF_FFFF);
        S7Server::clear_events(&store);
        assert!(S7Server::pick_event(&store).is_none());
    }

    #[tokio::test]
    async fn client_count_increments_on_connect() {
        let store = DataStore::new();
        let server = S7Server::bind(make_config()).await.unwrap();
        let addr = server.local_addr().unwrap();
        tokio::spawn(server.serve(store.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let params = snap7_client::types::ConnectParams::default();
        let _client = snap7_client::S7Client::connect(addr, params).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(S7Server::get_status(&store).clients_count, 1);
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
