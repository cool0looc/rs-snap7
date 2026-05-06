# rs-snap7

Pure-Rust, async implementation of the Siemens S7 protocol stack. Communicates with S7-300/400/1200/1500 PLCs over ISO-on-TCP without any native C dependency.

> **Status:** functional, pre-1.0. API may change.

## Workspace crates

| Crate | Description |
|---|---|
| [`snap7-client`](crates/snap7-client) | Async PLC client (`S7Client`, `S7PlusClient`, connection pool, TLS) — includes protocol layer |
| [`snap7-server`](crates/snap7-server) | In-process PLC simulator for testing |
| [`snap7-opcua-gateway`](crates/snap7-opcua-gateway) | OPC-UA bridge with subscription support |
| [`snap7-cli`](snap7-cli) | CLI binary (`snap7`) + helper servers |

---

## Benchmarks

Median latency (µs) — rs-snap7 vs python-snap7 vs libsnap7 (C), measured against an in-process simulator:

![Benchmark — rs-snap7 vs python-snap7 vs libsnap7](benchmark.png)

Rust sync is consistently **1.2–2.0× faster than python-snap7** and **up to 19× faster than libsnap7 (C)** at larger payload sizes.

---

## Install CLI binaries

```bash
# Main CLI
cargo install snap7-cli --bin snap7

# Test server (simulated PLC)
cargo install snap7-cli --bin snap7-test-server

# Sensor server (simulated PLC with live-updating REAL values)
cargo install snap7-cli --bin snap7-sensor-server

# OPC-UA gateway + demo tools (requires opcua feature)
cargo install snap7-cli --features opcua --bin gateway_demo
cargo install snap7-cli --features opcua --bin plc_batch_reader
cargo install snap7-cli --features opcua --bin opcua_subscriber
```

---

## Quick start

### Connect to a real PLC

```bash
# Read 16 bytes from DB1 at offset 0
snap7 -H 192.168.1.100 read --db 1 --offset 0 --size 16

# Write bytes (hex) to DB2 at offset 4
snap7 -H 192.168.1.100 write --db 2 --offset 4 --data DEADBEEF

# Read a typed tag
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB70,332.0       # bit access

# Watch a tag (poll every 500 ms, print on change only)
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 --interval-ms 500 --changes-only

# Upload a block
snap7 -H 192.168.1.100 block upload --type OB --number 1 --out ob1.bin

# List blocks
snap7 -H 192.168.1.100 block list

# Query SZL (system status list)
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0

# Run diagnostics
snap7 -H 192.168.1.100 diag
```

### Use the simulator locally

```bash
# Terminal 1 — start test server on port 10200
snap7-test-server

# Terminal 2 — read from it
snap7 -H 127.0.0.1 -p 10200 read --db 1 --offset 0 --size 4
# → DE AD BE EF
```

### Tag address syntax

```
DB<n>,<type><offset>
DB<n>,<offset>.<bit>
```

| Type | Width | Example |
|---|---|---|
| `REAL` | 4 bytes | `DB1,REAL0` |
| `DINT` | 4 bytes | `DB1,DINT4` |
| `DWORD` | 4 bytes | `DB1,DWORD4` |
| `INT` | 2 bytes | `DB1,INT8` |
| `WORD` | 2 bytes | `DB1,WORD8` |
| `BYTE` | 1 byte | `DB1,BYTE10` |
| bit | 1 bit | `DB1,332.0` |

### Output formats

```bash
snap7 -H 192.168.1.100 -f json  tag read DB1,REAL0
snap7 -H 192.168.1.100 -f csv   tag read DB1,REAL0
snap7 -H 192.168.1.100 -f text  tag read DB1,REAL0   # default
```

---

## Use as a library

Add to `Cargo.toml`:

```toml
[dependencies]
snap7-client = { git = "https://github.com/cool0looc/rs-snap7" }
```

### Async client

```rust
use snap7_client::{S7Client, ConnectParams};
use snap7_client::transport::TcpTransport;
use std::net::SocketAddr;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr: SocketAddr = "192.168.1.100:102".parse()?;
    let params = ConnectParams {
        rack: 0,
        slot: 1,
        connect_timeout: Duration::from_secs(5),
        ..Default::default()
    };

    let client = S7Client::<TcpTransport>::connect(addr, params).await?;

    // Read 4 bytes from DB1 offset 0
    let data = client.read_db(1, 0, 4).await?;
    println!("{data:?}");

    // Write
    client.write_db(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // Multi-read (one PDU)
    use snap7_client::MultiReadItem;
    let items = vec![
        MultiReadItem::db(1, 0, 4),
        MultiReadItem::db(2, 0, 2),
    ];
    let results = client.multi_read(items).await?;

    Ok(())
}
```

### Connection pool

```rust
use snap7_client::{S7Pool, PoolConfig, ConnectParams};
use std::net::SocketAddr;

let addr: SocketAddr = "192.168.1.100:102".parse()?;
let params = ConnectParams::default();
let config = PoolConfig { max_size: 4, ..Default::default() };

let pool = S7Pool::new(addr, params, config);
let client = pool.get().await?;
client.read_db(1, 0, 4).await?;
```

### Embed a test server

```rust
use snap7_server::{S7Server, ServerConfig, DataStore};
use std::net::SocketAddr;

let store = DataStore::new();
store.write_bytes(1, 0, &[1, 2, 3, 4]);

let cfg = ServerConfig {
    bind_addr: "127.0.0.1:0".parse()?,
    max_connections: 8,
};
let server = S7Server::bind(cfg).await?;
let addr = server.local_addr()?;

tokio::spawn(server.serve(store));
// addr is now ready to accept S7 connections
```

---

## OPC-UA gateway

The `snap7-opcua-gateway` crate (and `snap7 serve` command) bridges a PLC to OPC-UA clients with full subscription support.

```toml
# gateway.toml
plc_addr = "192.168.1.100:102"
opc_endpoint = "opc.tcp://0.0.0.0:4840"
poll_interval_ms = 500

[[tags]]
name = "Temperature"
tag = "DB1,REAL0"
writable = false

[[tags]]
name = "Setpoint"
tag = "DB2,REAL0"
writable = true
```

```bash
snap7 --features opcua serve --config gateway.toml
```

OPC-UA clients subscribe to `ns=2;s=Temperature` etc. and receive notifications at the polling interval. See [OPC-UA_SUBSCRIPTIONS.md](crates/snap7-opcua-gateway/OPC-UA_SUBSCRIPTIONS.md) for Python/Node.js subscription examples.

---

## Build from source

```bash
git clone https://github.com/cool0looc/rs-snap7
cd rs-snap7

# Build all
cargo build --release

# Build with OPC-UA gateway support
cargo build --release --features opcua -p snap7-cli

# Run tests
cargo test --workspace

# Run benchmarks
cargo bench -p snap7-bench
```

---

## License

MIT — see [LICENSE](LICENSE).
