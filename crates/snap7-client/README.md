# snap7-client

Async Rust client for Siemens S7 PLCs over ISO-on-TCP. Pure Rust — no FFI, no native C dependency.

Part of the [rs-snap7](https://github.com/cool0looc/rs-snap7) workspace.

## Features

| Capability | Status |
|---|---|
| **S7Comm** (S7-300/400) — read/write DB, multi-area, blocks, SZL | ✅ |
| **S7CommPlus** (S7-1200/1500 integrity mode) | ✅ |
| **Connection pool** (`S7Pool`) with RAII checkout | ✅ |
| **Multi-read / multi-write** with automatic PDU batching | ✅ |
| **Typed tag addressing** — DB, Merker, Timer, Counter | ✅ |
| **Tag read/write** with type decoding/encoding | ✅ |
| **Area absolute addressing** — `read_area` / `write_area` for any area | ✅ |
| **Block operations** — list, numbers, info, upload, download, delete, fill | ✅ |
| **PLC control** — stop, hot-start, cold-start, status (SZL 0x0424) | ✅ |
| **PLC information** — order code, CPU info, CP info, module list | ✅ |
| **Session password** — set, clear, read protection level | ✅ |
| **SZL queries** — system status list, clock, protection | ✅ |
| **Copy RAM → ROM, compress memory** | ✅ |
| **TLS transport** (S7CommPlus encrypted via `tokio-rustls`) | ✅ |
| **UDP transport** | ✅ |
| **Sync (blocking) API** — via `sync` feature | ✅ |
| **Pure Rust, zero native dependencies** | ✅ |

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

### Typed tag parsing

```rust
use snap7_client::tag::parse_tag;

// DB tags — comma or dot separator
let tag = parse_tag("DB1,REAL4")?;     // REAL at byte offset 4
let tag = parse_tag("DB170.REAL262")?; // dot separator, same result
let tag = parse_tag("DB70,332.0")?;    // bit 0 of byte 332

// Merker (M) tags — single-part, no separator
let tag = parse_tag("MB10")?;          // byte at offset 10
let tag = parse_tag("MW20")?;          // word at offset 20
let tag = parse_tag("MD4")?;           // dword at offset 4
let tag = parse_tag("M10.3")?;         // bit: byte 10, bit 3
let tag = parse_tag("MX5.7")?;         // bit: byte 5, bit 7 (MX prefix)

// Timer and Counter — element-index addressing
let tag = parse_tag("T5")?;            // Timer 5
let tag = parse_tag("C3")?;            // Counter 3
```

### Read/write any area

```rust
use snap7_client::proto::s7::header::{Area, TransportSize};

// Merker byte read
let data = client.read_area(Area::Marker, 0, 10, 1, TransportSize::Byte).await?;

// Timer read (element-index addressing, 2 bytes per element)
let data = client.read_area(Area::Timer, 0, 5, 1, TransportSize::Timer).await?;

// Counter read
let data = client.read_area(Area::Counter, 0, 3, 1, TransportSize::Counter).await?;

// Write Merker word
client.write_area(Area::Marker, 0, 20, TransportSize::Word, &[0x01, 0x00]).await?;
```

### PLC status

```rust
// Uses SZL 0x0424 — works on S7-300/400/1200/1500
let status = client.get_plc_status().await?;
// PlcStatus::Run | PlcStatus::Stop | PlcStatus::Unknown
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

DB tags use a comma or dot separator:

```
DB<n>,<type><byte-offset>
DB<n>.<type><byte-offset>   # dot separator, same result
DB<n>,<byte-offset>.<bit>   # bit access
```

Merker, Timer, Counter tags are single-part:

```
M<byte>.<bit>   MX<byte>.<bit>   MB<byte>   MW<byte>   MD<byte>
T<n>   C<n>
```

| Area | Type | Width | Example |
|---|---|---|---|
| DB | `REAL` | 4 B | `DB1,REAL0` |
| DB | `DINT` | 4 B | `DB1,DINT4` |
| DB | `DWORD` | 4 B | `DB1,DWORD4` |
| DB | `INT` | 2 B | `DB1,INT8` |
| DB | `WORD` | 2 B | `DB1,WORD8` |
| DB | `BYTE` | 1 B | `DB1,BYTE10` |
| DB | bit | 1 bit | `DB1,332.0` |
| Merker | bit | 1 bit | `M10.3` / `MX10.3` |
| Merker | byte | 1 B | `MB10` |
| Merker | word | 2 B | `MW20` |
| Merker | dword | 4 B | `MD4` |
| Timer | S5Time | 2 B | `T5` |
| Counter | BCD | 2 B | `C3` |

## License

MIT — see [LICENSE](../../LICENSE).
