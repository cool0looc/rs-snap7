# snap7-client

Async Rust client for Siemens S7 PLCs over ISO-on-TCP. Pure Rust — no FFI, no native C dependency.

Part of the [rs-snap7](https://github.com/cool0looc/rs-snap7) workspace.

## Features

- `S7Client` — async read/write for S7-300/400/1200/1500 via S7Comm
- `S7PlusClient` — S7CommPlus (S7-1200/1500 newer firmware)
- `S7Pool` — bounded connection pool with RAII checkout
- Multi-read / multi-write — batched PDU packing, automatic splitting at PDU limit
- Typed tag access — `DB1,REAL4`, `DB70,332.0` (bit), `DB1,INT8`, etc.
- TLS transport — encrypted S7CommPlus via `tokio-rustls`
- UDP transport
- `sync` feature — blocking wrapper around the async client

## Add to your project

```toml
[dependencies]
snap7-client = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick start

### Single connection

```rust
use snap7_client::{S7Client, ConnectParams};
use snap7_client::transport::TcpTransport;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr: SocketAddr = "192.168.1.100:102".parse()?;
    let params = ConnectParams { rack: 0, slot: 1, ..Default::default() };
    let client = S7Client::<TcpTransport>::connect(addr, params).await?;

    // Read 4 bytes from DB1 at offset 0
    let data = client.db_read(1, 0, 4).await?;
    println!("{data:x?}");

    // Write bytes
    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    Ok(())
}
```

### Multi-read (single PDU round-trip)

```rust
use snap7_client::MultiReadItem;

let items = vec![
    MultiReadItem::db(1, 0, 4),   // DB1, offset 0, 4 bytes
    MultiReadItem::db(2, 10, 2),  // DB2, offset 10, 2 bytes
];
let results = client.read_multi_vars(&items).await?;
// results[0] and results[1] — automatically batched across PDUs when needed
```

### Connection pool

```rust
use snap7_client::{S7Pool, PoolConfig, ConnectParams};
use std::net::SocketAddr;

let pool = S7Pool::new(
    "192.168.1.100:102".parse()?,
    ConnectParams::default(),
    PoolConfig { max_size: 4, ..Default::default() },
);

let guard = pool.acquire().await?;
guard.client().db_read(1, 0, 4).await?;
// connection returned to pool on drop
```

### Typed tag read/write

```rust
use snap7_client::tag::parse_tag;

let tag = parse_tag("DB1,REAL4")?;   // REAL at byte offset 4
let tag = parse_tag("DB1,DINT0")?;
let tag = parse_tag("DB70,332.0")?;  // bit 0 of byte 332
```

### TLS (S7CommPlus encrypted)

```rust
use snap7_client::tls::tls_connect;

let stream = tls_connect("plc.example.com", 102, None).await?;
let client = S7Client::from_transport(stream, ConnectParams::default()).await?;
```

### Sync (blocking) API

Enable the `sync` feature and use `snap7_client::client_sync::S7ClientSync`.

```toml
snap7-client = { version = "0.1", features = ["sync"] }
```

## Tag address syntax

```
DB<n>,<type><byte-offset>
DB<n>,<byte-offset>.<bit>
```

| Type | Width | Example |
|---|---|---|
| `REAL` | 4 B | `DB1,REAL0` |
| `DINT` | 4 B | `DB1,DINT4` |
| `DWORD` | 4 B | `DB1,DWORD4` |
| `INT` | 2 B | `DB1,INT8` |
| `WORD` | 2 B | `DB1,WORD8` |
| `BYTE` | 1 B | `DB1,BYTE10` |
| bit | 1 bit | `DB1,332.0` |

## License

MIT — see [LICENSE](../../LICENSE).
