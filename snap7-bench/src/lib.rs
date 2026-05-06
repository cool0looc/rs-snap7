use std::net::SocketAddr;
use snap7_client::{S7Client, transport::TcpTransport, types::ConnectParams};
use snap7_server::{DataStore, S7Server, ServerConfig};

/// Connect to an external server whose port is given by `SNAP7_BENCH_PORT`.
/// Panics if the env var is missing or the connection fails.
pub async fn connect_external() -> S7Client<TcpTransport> {
    let port: u16 = std::env::var("SNAP7_BENCH_PORT")
        .expect("SNAP7_BENCH_PORT must be set by the benchmark harness")
        .parse()
        .expect("SNAP7_BENCH_PORT must be a valid u16");
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let params = ConnectParams { rack: 0, slot: 1, ..ConnectParams::default() };
    S7Client::connect(addr, params).await.expect("connect to external server")
}

/// Spawn an in-process server (for standalone / unit use, not the comparison bench).
pub async fn spawn_server_and_client() -> (SocketAddr, S7Client<TcpTransport>) {
    let store = DataStore::new();
    store.write_bytes(1, 0, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

    let server = S7Server::bind(ServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        max_connections: 32,
    })
    .await
    .expect("bind server");

    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(store));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let params = ConnectParams { rack: 0, slot: 1, ..ConnectParams::default() };
    let client = S7Client::connect(addr, params).await.expect("connect client");
    (addr, client)
}
