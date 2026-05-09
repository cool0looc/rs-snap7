# rs-snap7

Pure-Rust, async implementation of the Siemens S7 protocol stack. Communicates with S7-300/400/1200/1500 PLCs over ISO-on-TCP without any native C dependency.

> **Status:** v0.1.7 — functional, pre-1.0. API may change.

## Features

| Capability | Status |
|---|---|
| **S7Comm** (S7-300/400) — read/write DB, multi-area, blocks, SZL | ✅ |
| **S7CommPlus** (S7-1200/1500 integrity mode) — read/write DB | ✅ |
| **TLS transport** (S7CommPlus encrypted mode) | ✅ |
| **UDP transport** | ✅ |
| **Connection pool** | ✅ |
| **Multi-read / multi-write** with automatic PDU batching | ✅ |
| **PLC control** — stop, hot-start, cold-start, status (via SZL 0x0424) | ✅ |
| **Memory management** — memory reset (`_MRES`), overall reset (`_OVERALL_RESET`) | ✅ |
| **PLC information** — order code, CPU info, CP info, module list | ✅ |
| **Block operations** — list, numbers, info, upload, full-upload, download, delete, fill, get, create DB | ✅ |
| **Block attributes** — set author, family, name, version on any block | ✅ |
| **Batch block upload** — upload all OB/FB/FC/DB in one call | ✅ |
| **Block CRC compare** — diff local block files against PLC (CRC-32) | ✅ |
| **Session password** — set, clear, read protection level | ✅ |
| **SZL queries** — system status list, SZL directory | ✅ |
| **PLC clock** — read and write (set clock, sync to host time) | ✅ |
| **Merker / Process I/O** — `mb_read/write`, `eb_read/write`, `ib_read/write` | ✅ |
| **Timer / Counter** — `tm_read/write`, `ct_read/write` | ✅ |
| **Force I/O** — force bits/bytes in I/Q areas, cancel force, read force list (SZL 0x0025) | ✅ |
| **PDU length query** | ✅ |
| **Copy RAM → ROM, compress memory** | ✅ |
| **Reconnect** — re-establish TCP + S7 handshake in-place | ✅ |
| **Exec time** — round-trip timing per command (ms) | ✅ |
| **CLI** — typed tags, block mgmt, clock, force, program compare, multi-format output | ✅ |
| **OPC-UA gateway** with subscription support | ✅ |
| **In-process PLC simulator** — data store, area lock/unlock, CPU state, event queue, callbacks | ✅ |
| **S7 Partner** (BSend/BRecv peer-to-peer, active + passive) | ✅ |

## Workspace crates

| Crate | Description |
|---|---|
| [`snap7-client`](crates/snap7-client) | Async PLC client (`S7Client`, `S7PlusClient`, connection pool, TLS, UDP) — includes protocol layer |
| [`snap7-server`](crates/snap7-server) | In-process PLC simulator with data store, area locking, event queue, CPU state, callbacks |
| [`snap7-partner`](crates/snap7-partner) | S7 Partner peer — active/passive BSend/BRecv over S7 UserData PDU |
| [`snap7-opcua-gateway`](crates/snap7-opcua-gateway) | OPC-UA bridge with subscription support |
| [`snap7-cli`](snap7-cli) | CLI binary (`snap7`) + helper servers |

---

## Feature matrix vs C snap7

### Client API (`Cli_*`)

| C snap7 function | rs-snap7 equivalent | Status |
|---|---|---|
| `Cli_Create` / `Cli_Destroy` | `S7Client::connect` / drop | ✅ |
| `Cli_Connect` / `Cli_ConnectTo` | `S7Client::connect(addr, params)` | ✅ |
| `Cli_Disconnect` | drop `S7Client` | ✅ |
| `Cli_GetConnected` | `is_connected()` | ✅ |
| — (reconnect after drop) | `reconnect()` on `S7Client<TcpTransport>` | ✅ |
| `Cli_SetConnectionParams` | `ConnectParams` struct | ✅ |
| `Cli_SetConnectionType` | `ConnectParams.rack/slot` | ✅ |
| `Cli_SetParam` / `Cli_GetParam` | `set_request_timeout` / `request_timeout`, `get_pdu_length` | ✅ |
| `Cli_DBRead` | `db_read(db, start, size)` | ✅ |
| `Cli_DBWrite` | `db_write(db, start, data)` | ✅ |
| `Cli_ABRead` | `ab_read(area, db, start, size)` | ✅ |
| `Cli_ABWrite` | `ab_write(area, db, start, data)` | ✅ |
| `Cli_MBRead` | `mb_read(start, size)` | ✅ |
| `Cli_MBWrite` | `mb_write(start, data)` | ✅ |
| `Cli_EBRead` | `eb_read(start, size)` | ✅ |
| `Cli_EBWrite` | `eb_write(start, data)` | ✅ |
| `Cli_IBRead` (Process Output) | `ib_read(start, size)` | ✅ |
| `Cli_IBWrite` (Process Output) | `ib_write(start, data)` | ✅ |
| `Cli_TMRead` | `tm_read(start, count)` | ✅ |
| `Cli_TMWrite` | `tm_write(start, data)` | ✅ |
| `Cli_CTRead` | `ct_read(start, count)` | ✅ |
| `Cli_CTWrite` | `ct_write(start, data)` | ✅ |
| `Cli_ReadArea` | `read_area(area, db, start, count, transport)` | ✅ |
| `Cli_WriteArea` | `write_area(area, db, start, transport, data)` | ✅ |
| `Cli_ReadMultiVars` | `read_multi_vars(items)` | ✅ |
| `Cli_WriteMultiVars` | `write_multi_vars(items)` | ✅ |
| `Cli_DBGet` | `db_get(db)` | ✅ |
| `Cli_DBFill` | `db_fill(db, fill_byte)` | ✅ |
| `Cli_ListBlocks` | `list_blocks()` | ✅ |
| `Cli_ListBlocksOfType` | `list_blocks_of_type(block_type)` | ✅ |
| `Cli_GetAgBlockInfo` | `get_ag_block_info(block_type, block_num)` | ✅ |
| `Cli_GetPgBlockInfo` | `S7Client::parse_block_info(data)` (offline, no PLC needed) | ✅ |
| `Cli_Upload` | `upload(block_type, block_num)` | ✅ |
| `Cli_FullUpload` | `full_upload(block_type, block_num)` | ✅ |
| `Cli_Download` | `download(block_type, block_num, data)` | ✅ |
| `Cli_Delete` | `delete_block(block_type, block_num)` | ✅ |
| `Cli_PlcStop` | `plc_stop()` | ✅ |
| `Cli_PlcHotStart` | `plc_hot_start()` | ✅ |
| `Cli_PlcColdStart` | `plc_cold_start()` | ✅ |
| `Cli_GetPlcStatus` | `get_plc_status()` | ✅ |
| `Cli_GetPlcDateTime` | `read_clock()` | ✅ |
| `Cli_SetPlcDateTime` | `set_clock(dt)` | ✅ |
| `Cli_SetPlcSystemDateTime` | `set_clock_to_now()` | ✅ |
| `Cli_GetOrderCode` | `get_order_code()` | ✅ |
| `Cli_GetCpuInfo` | `get_cpu_info()` | ✅ |
| `Cli_GetCpInfo` | `get_cp_info()` | ✅ |
| `Cli_ReadSZL` | `read_szl(szl_id, szl_index)` | ✅ |
| `Cli_ReadSZLList` | `read_szl_list()` | ✅ |
| `Cli_SetSessionPassword` | `set_session_password(pw)` | ✅ |
| `Cli_ClearSessionPassword` | `clear_session_password()` | ✅ |
| `Cli_GetProtection` | `get_protection()` | ✅ |
| `Cli_CopyRamToRom` | `copy_ram_to_rom()` | ✅ |
| `Cli_Compress` | `compress()` | ✅ |
| `Cli_GetPduLength` | `get_pdu_length()` | ✅ |
| `Cli_GetExecTime` | `get_exec_time()` → ms since last send/recv | ✅ |
| `Cli_GetLastError` | Rust `Result<T, Error>` | ✅ |
| `Cli_ErrorText` | `Error::to_string()` | ✅ |
| `Cli_IsoExchangeBuffer` | — (raw PDU exchange) | ❌ |
| `Cli_As*` (all async variants) | native `async fn` everywhere | ✅ |
| `Cli_WaitAsCompletion` / `Cli_CheckAsCompletion` | `.await` | ✅ |
| `Cli_SetAsCallback` | `.await` / tokio tasks | ✅ |
| — (no C equivalent) | `memory_reset()` — PI service `_MRES` | ✅ |
| — (no C equivalent) | `overall_reset()` — PI service `_OVERALL_RESET` | ✅ |
| — (no C equivalent) | `force_bit(area, byte, bit, value)` | ✅ |
| — (no C equivalent) | `force_byte(area, byte, value)` | ✅ |
| — (no C equivalent) | `force_cancel_byte(area, byte)` | ✅ |
| — (no C equivalent) | `read_force_list()` — SZL 0x0025 | ✅ |
| — (no C equivalent) | `upload_all_blocks(&[types])` — batch upload | ✅ |
| — (no C equivalent) | `create_db(num, size, attrs)` | ✅ |
| — (no C equivalent) | `compare_blocks(local, report_plc_only)` — CRC-32 diff | ✅ |
| — (no C equivalent) | `BlockData::set_attributes(attrs)` — author/family/name/version | ✅ |

### Server API (`Srv_*`)

| C snap7 function | rs-snap7 equivalent | Status |
|---|---|---|
| `Srv_Create` / `Srv_Destroy` | `S7Server::bind(cfg)` / drop | ✅ |
| `Srv_Start` / `Srv_StartTo` | `server.serve(store)` / `S7Server::start_to(addr, max_conn)` | ✅ |
| `Srv_Stop` | drop / cancel the serve task | ✅ |
| `Srv_RegisterArea` | `store.register_area(area_code, size)` | ✅ |
| `Srv_UnregisterArea` | `store.unregister_area(area_code)` | ✅ |
| `Srv_LockArea` | `store.lock_area(area_code)` / `S7Server::lock_area` | ✅ |
| `Srv_UnlockArea` | `store.unlock_area(area_code)` / `S7Server::unlock_area` | ✅ |
| `Srv_GetStatus` | `S7Server::get_status(store)` → `ServerStatus` | ✅ |
| `Srv_SetCpuStatus` | `S7Server::set_cpu_status(store, state)` | ✅ |
| `Srv_GetMask` | `S7Server::get_mask(store)` | ✅ |
| `Srv_SetMask` | `S7Server::set_mask(store, mask)` | ✅ |
| `Srv_ClearEvents` | `S7Server::clear_events(store)` | ✅ |
| `Srv_PickEvent` | `S7Server::pick_event(store)` → `Option<EventInfo>` | ✅ |
| `Srv_SetEventsCallback` | `store.on_event(cb)` | ✅ |
| `Srv_SetReadEventsCallback` | `store.on_read(cb)` | ✅ |
| `Srv_SetRWAreaCallback` | `store.on_write(cb)` | ✅ |
| `Srv_GetParam` / `Srv_SetParam` | `ServerConfig` struct | ✅ |
| `Srv_ErrorText` / `Srv_EventText` | `Error::to_string()` | ✅ |

### Partner API (`Par_*`)

| C snap7 function | rs-snap7 equivalent | Status |
|---|---|---|
| `Par_Create` / `Par_Destroy` | `S7Partner::connect` / `S7Partner::listen` / drop | ✅ |
| `Par_Start` / `Par_StartTo` | `S7Partner::connect(addr)` / `S7Partner::listen(addr)` | ✅ |
| `Par_Stop` | drop | ✅ |
| `Par_BSend` | `partner.bsend(r_id, data)` | ✅ |
| `Par_AsBSend` | native `async fn bsend` | ✅ |
| `Par_WaitAsBSendCompletion` | `.await` | ✅ |
| `Par_CheckAsBSendCompletion` | `.await` + `tokio::select!` | ✅ |
| `Par_SetSendCallback` | tokio tasks / channels | ✅ |
| `Par_BRecv` | `partner.brecv()` | ✅ |
| `Par_CheckAsBRecvCompletion` | `.await` + `tokio::select!` | ✅ |
| `Par_SetRecvCallback` | tokio tasks / channels | ✅ |
| `Par_GetStatus` | `partner.get_status()` → `PartnerStatus` | ✅ |
| `Par_GetTimes` | — (no per-direction timing) | ❌ |
| `Par_GetStats` | `partner.get_stats()` → `(bytes_sent, bytes_recv)` | ✅ |
| `Par_GetParam` / `Par_SetParam` | `recv_timeout` / `send_timeout` fields | ✅ |
| `Par_GetLastError` | Rust `Result<T, Error>` | ✅ |
| `Par_ErrorText` | `Error::to_string()` | ✅ |

### rs-snap7 extras (no C equivalent)

| Feature | Crate |
|---|---|
| S7CommPlus (S7-1200/1500 integrity mode) | `snap7-client` |
| TLS transport (S7CommPlus encrypted) | `snap7-client` |
| UDP transport | `snap7-client` |
| Connection pool with max-size semaphore | `snap7-client` |
| Memory reset / overall reset (PI services) | `snap7-client` |
| Force I/O bits and bytes (I/Q areas) | `snap7-client` |
| Batch block upload (all OB/FB/FC/DB in one call) | `snap7-client` |
| Create DB with custom size and attributes | `snap7-client` |
| Block CRC-32 compare (local vs PLC) | `snap7-client` |
| Block attribute editing (author/family/name/version) | `snap7-client` |
| Reconnect in-place (TCP + S7 handshake) | `snap7-client` |
| Per-command exec time (ms, send→recv) | `snap7-client` |
| OPC-UA gateway with subscriptions | `snap7-opcua-gateway` |
| Typed tag parser (DB/M/T/C address syntax) | `snap7-cli` |
| CLI with JSON/CSV/text output | `snap7-cli` |
| CLI clock — read/set/sync with --force | `snap7-cli` |
| CLI force — set/cancel I/Q bits and bytes, list | `snap7-cli` |
| CLI program — mem-reset, format, batch-upload, compare | `snap7-cli` |
| Fully async — no blocking thread per connection | all crates |

---

## Protocol analysis — known gaps

The following S7 protocol features exist in the standard or in C snap7 but are **not yet implemented**:

| Feature | Protocol detail | Priority |
|---|---|---|
| `Cli_IsoExchangeBuffer` | Raw PDU exchange — sends arbitrary TPDU, receives response | Low |
| `Par_GetTimes` | Per-direction BSend/BRecv timing statistics | Low |
| **True force table** (persistent I forcing) | CPU force table via UserData `grProgram` — allows forcing inputs that persist across scan cycles | Medium |
| **SZL 0x0092** (diagnostic buffer) | Reads the CPU diagnostic ring buffer (fault entries) | Medium |
| **SZL 0x0111** (module status) | Returns installed module count and status per slot | Low |
| **SZL 0x0B00** / **0x0B01** (communication status) | Connection table, active sessions per rack/slot | Low |
| **PI service `_WRK_COMPRESS`** | Another memory compress variant (supplement to `Cli_Compress`) | Low |
| **PI service `_INSMOD` / `_DELMOD`** | Insert / delete modules (S7-400 hardware config) | Low |
| **Multi-block download** | Download multiple blocks in one S7 session without reconnect | Medium |
| **Block existence check** | Lightweight "does block X exist?" without full info query | Low |
| **Online block modification** | Modify a running DB online (S7-400/1500 only) | Low |
| **S7CommPlus multi-read** | Batch reads in S7+ protocol (S7-1500 integrity mode) | Medium |
| **UDT / SDB upload** | Upload user-defined types and system DBs | Low |
| **Force via peripheral area (0x80)** | `Area::PeripheralInput` write — direct hardware register access | Medium |

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

# Read a typed tag (DB)
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB70,332.0       # bit access
snap7 -H 192.168.1.100 tag read DB170,REAL262     # comma separator
snap7 -H 192.168.1.100 tag read DB170.REAL262     # dot separator (same)

# Read Merker (M), Timer (T), Counter (C) tags
snap7 -H 192.168.1.100 tag read MB10              # Merker byte
snap7 -H 192.168.1.100 tag read MW20              # Merker word
snap7 -H 192.168.1.100 tag read MD4               # Merker dword
snap7 -H 192.168.1.100 tag read M10.3             # Merker bit (byte 10, bit 3)
snap7 -H 192.168.1.100 tag read MX5.7             # Merker bit (MX prefix)
snap7 -H 192.168.1.100 tag read T5                # Timer 5
snap7 -H 192.168.1.100 tag read C3                # Counter 3

# Write a typed tag
snap7 -H 192.168.1.100 tag write DB1,REAL0 3.14
snap7 -H 192.168.1.100 tag write DB10,DINT0 42
snap7 -H 192.168.1.100 tag write MB10 255

# Watch a tag (poll every 500 ms, print on change only)
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 --interval-ms 500 --changes-only

# Block operations
snap7 -H 192.168.1.100 block list
snap7 -H 192.168.1.100 block numbers --type DB
snap7 -H 192.168.1.100 block info --type DB --number 1
snap7 -H 192.168.1.100 block upload --type DB --number 1 --out db1.bin
snap7 -H 192.168.1.100 block download --type DB --number 1 --file db1.bin
snap7 -H 192.168.1.100 block create-db --number 50 --size 512 --author Kyle --family MYAPP --version 1.0
snap7 -H 192.168.1.100 block set-attrs --type DB --number 50 --author Kyle --version 2.0

# Program management
snap7 -H 192.168.1.100 program batch-upload --types OB,FB,FC,DB --out ./blocks --full
snap7 -H 192.168.1.100 program compare --dir ./blocks --plc-only
snap7 -H 192.168.1.100 program mem-reset --force     # clears work memory (PLC must STOP)
snap7 -H 192.168.1.100 program format --force        # full memory wipe

# PLC clock
snap7 -H 192.168.1.100 clock read
snap7 -H 192.168.1.100 clock set 2025-01-15T10:30:00 --force
snap7 -H 192.168.1.100 clock sync --force            # sync to system time

# Force I/O
snap7 -H 192.168.1.100 force set Q0.3 1             # force output bit
snap7 -H 192.168.1.100 force set QB2 0xFF           # force output byte
snap7 -H 192.168.1.100 force cancel QB0             # cancel force
snap7 -H 192.168.1.100 force list                   # SZL 0x0025 force table

# Query SZL (system status list)
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0

# PLC control
snap7 -H 192.168.1.100 plc-control status
snap7 -H 192.168.1.100 plc-control stop
snap7 -H 192.168.1.100 plc-control hotstart
snap7 -H 192.168.1.100 plc-control coldstart

# PLC information
snap7 -H 192.168.1.100 info order-code
snap7 -H 192.168.1.100 info cpu-info
snap7 -H 192.168.1.100 info cp-info
snap7 -H 192.168.1.100 info module-list

# Session password
snap7 -H 192.168.1.100 password set mypass
snap7 -H 192.168.1.100 password clear

# Run diagnostics
snap7 -H 192.168.1.100 diag
```

Every command prints the round-trip execution time to stderr on completion:

```
exec time: 4 ms
```

### TLS and UDP transport

```bash
# S7CommPlus with TLS (S7-1200/1500)
snap7 -H 192.168.1.100 --tls read --db 1 --offset 0 --size 4

# With custom CA certificate
snap7 -H 192.168.1.100 --tls --tls-ca /path/to/ca.pem read --db 1 --offset 0 --size 4

# UDP transport (ISO-on-UDP)
snap7 -H 192.168.1.100 --udp read --db 1 --offset 0 --size 4
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

DB tags require a comma (or dot) separator between the DB number and the type:

```
DB<n>,<type><offset>
DB<n>.<type><offset>    # dot separator, same result
DB<n>,<offset>.<bit>    # bit access
```

Merker, Timer, and Counter tags are single-part — no separator needed:

```
M<byte>.<bit>           # Merker bit  (e.g. M10.3)
MX<byte>.<bit>          # Merker bit  (MX prefix)
MB<byte>                # Merker byte (e.g. MB10)
MW<byte>                # Merker word (e.g. MW20)
MD<byte>                # Merker dword(e.g. MD4)
T<n>                    # Timer       (e.g. T5)
C<n>                    # Counter     (e.g. C3)
```

| Area | Type | Width | Example |
|---|---|---|---|
| DB | `REAL` | 4 bytes | `DB1,REAL0` |
| DB | `DINT` | 4 bytes | `DB1,DINT4` |
| DB | `DWORD` | 4 bytes | `DB1,DWORD4` |
| DB | `INT` | 2 bytes | `DB1,INT8` |
| DB | `WORD` | 2 bytes | `DB1,WORD8` |
| DB | `BYTE` | 1 byte | `DB1,BYTE10` |
| DB | bit | 1 bit | `DB1,332.0` |
| Merker | bit | 1 bit | `M10.3` / `MX10.3` |
| Merker | byte | 1 byte | `MB10` |
| Merker | word | 2 bytes | `MW20` |
| Merker | dword | 4 bytes | `MD4` |
| Timer | S5Time | 2 bytes | `T5` |
| Counter | BCD | 2 bytes | `C3` |

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
snap7-client  = { git = "https://github.com/cool0looc/rs-snap7" }
snap7-server  = { git = "https://github.com/cool0looc/rs-snap7" }  # optional
snap7-partner = { git = "https://github.com/cool0looc/rs-snap7" }  # optional
```

### Async client (S7Comm — S7-300/400)

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

    // Read / write DB
    let data = client.db_read(1, 0, 4).await?;
    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // Merker / process I/O
    let _mk = client.mb_read(10, 4).await?;
    client.mb_write(10, &[0xFF]).await?;
    let _pi = client.eb_read(0, 2).await?;
    let _po = client.ib_read(0, 2).await?;

    // Timers and counters
    let _timers   = client.tm_read(0, 5).await?;
    let _counters = client.ct_read(0, 3).await?;

    // Multi-read / multi-write (one PDU, automatic batching)
    use snap7_client::{MultiReadItem, MultiWriteItem};
    let items = vec![MultiReadItem::db(1, 0, 4), MultiReadItem::db(2, 0, 2)];
    let _results = client.read_multi_vars(&items).await?;

    let items = vec![MultiWriteItem::db(1, 0, vec![0xAA, 0xBB])];
    client.write_multi_vars(&items).await?;

    // Read/write any area with explicit transport size
    use snap7_client::proto::s7::header::{Area, TransportSize};
    let _data = client.read_area(Area::Marker, 0, 10, 1, TransportSize::Byte).await?;
    let _data = client.read_area(Area::Timer, 0, 5, 1, TransportSize::Timer).await?;
    client.write_area(Area::Marker, 0, 10, TransportSize::Byte, &[0xFF]).await?;

    // PDU length
    let pdu = client.get_pdu_length().await;
    println!("Negotiated PDU: {pdu} bytes");

    Ok(())
}
```

### S7CommPlus client (S7-1200/1500 integrity mode)

```rust
use snap7_client::S7PlusClient;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr: SocketAddr = "192.168.1.100:102".parse()?;
    let client = S7PlusClient::connect(addr, Default::default()).await?;

    let data = client.db_read(1, 0, 4).await?;
    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // Multi-read
    use snap7_client::plus_client::DbVarSpec;
    let specs = vec![DbVarSpec { db: 1, offset: 0, length: 4 }];
    let _results = client.read_multi_vars(&specs).await?;

    // TLS connection
    let _client = S7PlusClient::connect_tls(
        addr, "plc.example.com", None, Default::default()
    ).await?;

    Ok(())
}
```

### PLC control & information

```rust
// Status
let status = client.get_plc_status().await?;  // Run | Stop | Unknown
client.plc_stop().await?;
client.plc_hot_start().await?;
client.plc_cold_start().await?;

// Memory management (PLC must be in STOP)
client.memory_reset().await?;     // clears work memory
client.overall_reset().await?;   // wipes load + work + retain

// Identity
let oc = client.get_order_code().await?;    // e.g. "6ES7 317-2EK14-0AB0"
let ci = client.get_cpu_info().await?;
let cp = client.get_cp_info().await?;

// Clock
let dt = client.read_clock().await?;
client.set_clock(&dt).await?;
client.set_clock_to_now().await?;           // sync PLC clock to host UTC

// SZL
let szl  = client.read_szl(0x001C, 0).await?;
let ids  = client.read_szl_list().await?;   // all available SZL IDs
```

### Block operations

```rust
let list    = client.list_blocks().await?;
let numbers = client.list_blocks_of_type(0x41).await?;    // all DBs
let info    = client.get_ag_block_info(0x41, 1).await?;   // DB 1
let raw     = client.db_get(1).await?;

let data    = client.upload(0x41, 1).await?;               // header only
let mc7     = client.full_upload(0x41, 1).await?;          // with MC7 code
client.download(0x41, 1, &data).await?;
client.delete_block(0x41, 1).await?;
client.db_fill(1, 0x00).await?;

// Create empty DB with attributes
use snap7_client::BlockAttributes;
let attrs = BlockAttributes {
    author: Some("Kyle".into()),
    family: Some("MYAPP".into()),
    name: Some("Config".into()),
    version: Some(0x10),  // 1.0
    flags: None,
};
client.create_db(50, 512, Some(&attrs)).await?;

// Batch upload all OB/FB/FC/DB
let all = client.upload_all_blocks(&[0x38, 0x45, 0x43, 0x41]).await?;
for (bt, num, data) in &all {
    println!("{}{}: {} bytes", snap7_client::block_type_name(*bt), num, data.len());
}

// CRC-32 compare local files vs PLC
let local = vec![(0x41u8, 1u16, std::fs::read("DB1.bin")?)];
let results = client.compare_blocks(&local, true).await?;
for (bt, num, result) in results {
    println!("{}{}: {:?}", snap7_client::block_type_name(bt), num, result);
}

// Set block attributes (upload → modify → re-download)
use snap7_client::BlockData;
let raw = client.full_upload(0x41, 1).await?;
if let Some(mut block) = BlockData::from_bytes(&raw) {
    block.set_attributes(&attrs);
    client.download(0x41, 1, &block.to_bytes()).await?;
}
```

### Force I/O

```rust
use snap7_client::proto::s7::header::Area;

// Force output Q0.3 = 1
client.force_bit(Area::ProcessOutput, 0, 3, true).await?;

// Force entire output byte QB2 = 0xFF
client.force_byte(Area::ProcessOutput, 2, 0xFF).await?;

// Cancel force on QB0
client.force_cancel_byte(Area::ProcessOutput, 0).await?;

// Read force table (SZL 0x0025)
let force_data = client.read_force_list().await?;
```

### Session password & protection

```rust
client.set_session_password("mypass").await?;
client.clear_session_password().await?;
let prot = client.get_protection().await?;
```

### Connection pool

```rust
use snap7_client::{S7Pool, PoolConfig, ConnectParams};

let pool   = S7Pool::new(addr, ConnectParams::default(), PoolConfig { max_size: 4, ..Default::default() });
let client = pool.get().await?;
client.db_read(1, 0, 4).await?;
```

### Embed a test server

```rust
use snap7_server::{S7Server, ServerConfig, DataStore, CpuState};
use snap7_server::store::area;

let store = DataStore::new();
store.write_bytes(1, 0, &[1, 2, 3, 4]);
store.register_area(area::PROCESS_INPUTS, 1024);
store.set_cpu_state(CpuState::Run);

// Area locking — writes silently ignored while locked
store.lock_area(area::DATA_BLOCK);
store.unlock_area(area::DATA_BLOCK);

// Event queue
store.set_mask(0xFFFF_FFFF);
if let Some(ev) = store.pick_event() {
    println!("{}: area=0x{:02X}", ev.event, ev.area);
}

// Status
let status = S7Server::get_status(&store);
println!("clients={} cpu={:?}", status.clients_count, status.cpu_state);

// Callbacks
store.on_read(|info|  println!("read  area=0x{:02X} db={}", info.area, info.db_number));
store.on_write(|info| println!("write area=0x{:02X} db={}", info.area, info.db_number));
store.on_event(|ev|   println!("event: {ev}"));

let server = S7Server::bind(ServerConfig {
    bind_addr: "127.0.0.1:0".parse()?,
    max_connections: 8,
}).await?;
tokio::spawn(server.serve(store));
```

### S7 Partner (BSend/BRecv)

```rust
use snap7_partner::S7Partner;
use std::net::SocketAddr;

// Active partner — connects to remote
let active = S7Partner::connect("192.168.1.100:102".parse()?).await?;
active.bsend(0x0000_0001, b"hello").await?;

// Passive partner — listens for remote to connect
let passive = S7Partner::listen("0.0.0.0:102".parse()?).await?;
let (r_id, data) = passive.brecv().await?;
println!("R_ID={r_id:#010x} data={data:?}");

// Bind separately to retrieve the port first
let (listener, partner) = S7Partner::bind("127.0.0.1:0".parse()?).await?;
let _port = listener.local_addr()?.port();
S7Partner::accept(&partner, &listener).await?;
let (r_id, data) = partner.brecv().await?;
```

### UDP transport

```rust
use snap7_client::{S7Client, ConnectParams, UdpTransport};

let client = S7Client::<UdpTransport>::connect_udp(addr, ConnectParams::default()).await?;
let data = client.db_read(1, 0, 4).await?;
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
