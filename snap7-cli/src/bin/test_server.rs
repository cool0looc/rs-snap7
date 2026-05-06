use snap7_server::{DataStore, S7Server, ServerConfig};

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10200);

    let store = DataStore::new();
    // DB1[0..4] = DE AD BE EF  (read test)
    store.write_bytes(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
    // DB3[0..4] = 01 02 03 04  (pre-populated read test)
    store.write_bytes(3, 0, &[0x01, 0x02, 0x03, 0x04]);

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let cfg = ServerConfig {
        bind_addr: addr,
        max_connections: 8,
    };
    let server = S7Server::bind(cfg).await.expect("failed to bind S7Server");

    eprintln!("snap7-test-server listening on 127.0.0.1:{port}");

    tokio::select! {
        r = server.serve(store) => { eprintln!("server exited: {r:?}"); }
        _ = tokio::signal::ctrl_c() => { eprintln!("shutting down"); }
    }
}
