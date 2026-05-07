# snap7-server

In-process S7 PLC simulator for testing and development. Pure Rust — no FFI, no native C dependency. Responds to real S7Comm read/write requests over TCP.

Part of the [rs-snap7](https://github.com/cool0looc/rs-snap7) workspace.

## Features

| Capability | Status |
|---|---|
| **S7Comm read/write dispatch** — handles `ReadVar` / `WriteVar` requests | ✅ |
| **Multi-item read/write** — single-PDU multi-area operations | ✅ |
| **DataStore** — thread-safe in-memory store by `(area, db, offset)` | ✅ |
| **Area registration** — PI, PA, MK, DB, timer, counter, etc. | ✅ |
| **CPU state tracking** — RUN / STOP with state-change callbacks | ✅ |
| **Read / write event callbacks** | ✅ |
| **Max connections limit** | ✅ |
| **Ephemeral port support** — `127.0.0.1:0` for tests | ✅ |
| **Pure Rust, zero native dependencies** | ✅ |

## Use cases

## Add to your project

```toml
[dev-dependencies]
snap7-server = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick start

```rust
use snap7_server::{S7Server, ServerConfig, DataStore};
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Pre-seed data block 1
    let store = DataStore::new();
    store.write_bytes(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]);

    let cfg = ServerConfig {
        bind_addr: "127.0.0.1:10200".parse()?,
        max_connections: 8,
    };
    let server = S7Server::bind(cfg).await?;
    println!("listening on {}", server.local_addr()?);

    server.serve(store).await?;
    Ok(())
}
```

### Ephemeral port (tests)

```rust
let cfg = ServerConfig {
    bind_addr: "127.0.0.1:0".parse()?,  // OS assigns a free port
    max_connections: 4,
};
let server = S7Server::bind(cfg).await?;
let addr = server.local_addr()?;
tokio::spawn(server.serve(store));

// connect snap7-client to `addr`
```

## API

| Type | Purpose |
|---|---|
| `S7Server` | Binds TCP socket, accepts connections |
| `ServerConfig` | `bind_addr` + `max_connections` |
| `DataStore` | Thread-safe in-memory data block store; read/write by `(db, offset)` |

## Notes

- Implements TPKT / COTP / S7Comm handshake and dispatch.
- Supports multi-item read/write (`ReadVar` / `WriteVar`).
- Does not implement S7CommPlus, SZL, or block upload — those return protocol errors.

## License

MIT — see [LICENSE](../../LICENSE).
