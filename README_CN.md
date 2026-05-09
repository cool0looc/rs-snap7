# rs-snap7

纯 Rust 异步实现的西门子 S7 协议栈。通过 ISO-on-TCP 与 S7-300/400/1200/1500 系列 PLC 通信，无需任何 C 原生依赖。

> **状态：** v0.1.7 — 功能完整，1.0 前版本，API 可能变动。

## 功能特性

| 能力 | 状态 |
|---|---|
| **S7Comm**（S7-300/400）— 读写 DB、多区域、块操作、SZL | ✅ |
| **S7CommPlus**（S7-1200/1500 完整性模式）— 读写 DB | ✅ |
| **TLS 传输**（S7CommPlus 加密模式） | ✅ |
| **UDP 传输** | ✅ |
| **连接池** | ✅ |
| **多读 / 多写**，支持自动 PDU 分批 | ✅ |
| **PLC 控制** — 停止、热启动、冷启动、状态查询 | ✅ |
| **PLC 信息** — 订货号、CPU 信息、CP 信息、模块列表 | ✅ |
| **块操作** — 列表、编号、信息、上传、完整上传、下载、删除、填充、获取 | ✅ |
| **会话密码** — 设置、清除、读取保护级别 | ✅ |
| **SZL 查询** — 系统状态列表、SZL 目录 | ✅ |
| **PLC 时钟** — 读取与写入（设置时钟、同步到主机时间） | ✅ |
| **Merker / 过程 I/O** — `mb_read/write`、`eb_read/write`、`ib_read/write` | ✅ |
| **定时器 / 计数器** — `tm_read/write`、`ct_read/write` | ✅ |
| **PDU 长度查询** | ✅ |
| **拷贝 RAM → ROM、压缩内存** | ✅ |
| **CLI** — 类型化标签（DB/Merker/定时器/计数器），多格式输出（text/json/csv） | ✅ |
| **OPC-UA 网关**，支持订阅 | ✅ |
| **进程内 PLC 仿真器** — 数据存储、区域锁定/解锁、CPU 状态、事件队列、回调 | ✅ |
| **S7 Partner**（BSend/BRecv 点对点，主动 + 被动） | ✅ |

## 工作区 crate

| Crate | 说明 |
|---|---|
| [`snap7-client`](crates/snap7-client) | 异步 PLC 客户端（`S7Client`、`S7PlusClient`、连接池、TLS、UDP）—— 包含协议层 |
| [`snap7-server`](crates/snap7-server) | 进程内 PLC 仿真器，含数据存储、区域锁定、事件队列、CPU 状态、回调 |
| [`snap7-partner`](crates/snap7-partner) | S7 Partner 节点 —— 通过 S7 UserData PDU 实现主动/被动 BSend/BRecv |
| [`snap7-opcua-gateway`](crates/snap7-opcua-gateway) | 支持订阅的 OPC-UA 网关 |
| [`snap7-cli`](snap7-cli) | CLI 二进制（`snap7`）及辅助服务器 |

---

## 与 C snap7 功能对比矩阵

### 客户端 API（`Cli_*`）

| C snap7 函数 | rs-snap7 对应 | 状态 |
|---|---|---|
| `Cli_Create` / `Cli_Destroy` | `S7Client::connect` / drop | ✅ |
| `Cli_Connect` / `Cli_ConnectTo` | `S7Client::connect(addr, params)` | ✅ |
| `Cli_Disconnect` | drop `S7Client` | ✅ |
| `Cli_GetConnected` | `is_connected()` | ✅ |
| —（断线重连） | `S7Client<TcpTransport>` 的 `reconnect()` | ✅ |
| `Cli_SetConnectionParams` | `ConnectParams` 结构体 | ✅ |
| `Cli_SetConnectionType` | `ConnectParams.rack/slot` | ✅ |
| `Cli_SetParam` / `Cli_GetParam` | `set_request_timeout` / `request_timeout`、`get_pdu_length` | ✅ |
| `Cli_DBRead` | `db_read(db, start, size)` | ✅ |
| `Cli_DBWrite` | `db_write(db, start, data)` | ✅ |
| `Cli_ABRead` | `ab_read(area, db, start, size)` | ✅ |
| `Cli_ABWrite` | `ab_write(area, db, start, data)` | ✅ |
| `Cli_MBRead` | `mb_read(start, size)` | ✅ |
| `Cli_MBWrite` | `mb_write(start, data)` | ✅ |
| `Cli_EBRead` | `eb_read(start, size)` | ✅ |
| `Cli_EBWrite` | `eb_write(start, data)` | ✅ |
| `Cli_IBRead`（过程输出） | `ib_read(start, size)` | ✅ |
| `Cli_IBWrite`（过程输出） | `ib_write(start, data)` | ✅ |
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
| `Cli_GetPgBlockInfo` | `S7Client::parse_block_info(data)`（离线，无需 PLC） | ✅ |
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
| `Cli_GetExecTime` | `get_exec_time()` → 上次操作耗时（毫秒） | ✅ |
| `Cli_GetLastError` | Rust `Result<T, Error>` | ✅ |
| `Cli_ErrorText` | `Error::to_string()` | ✅ |
| `Cli_IsoExchangeBuffer` | —（原始 PDU 交换） | ❌ |
| `Cli_As*`（所有异步变体） | 原生 `async fn` | ✅ |
| `Cli_WaitAsCompletion` / `Cli_CheckAsCompletion` | `.await` | ✅ |
| `Cli_SetAsCallback` | `.await` / tokio 任务 | ✅ |

### 服务端 API（`Srv_*`）

| C snap7 函数 | rs-snap7 对应 | 状态 |
|---|---|---|
| `Srv_Create` / `Srv_Destroy` | `S7Server::bind(cfg)` / drop | ✅ |
| `Srv_Start` / `Srv_StartTo` | `server.serve(store)` / `S7Server::start_to(addr, max_conn)` | ✅ |
| `Srv_Stop` | drop / 取消 serve 任务 | ✅ |
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
| `Srv_GetParam` / `Srv_SetParam` | `ServerConfig` 结构体 | ✅ |
| `Srv_ErrorText` / `Srv_EventText` | `Error::to_string()` | ✅ |

### Partner API（`Par_*`）

| C snap7 函数 | rs-snap7 对应 | 状态 |
|---|---|---|
| `Par_Create` / `Par_Destroy` | `S7Partner::connect` / `S7Partner::listen` / drop | ✅ |
| `Par_Start` / `Par_StartTo` | `S7Partner::connect(addr)` / `S7Partner::listen(addr)` | ✅ |
| `Par_Stop` | drop | ✅ |
| `Par_BSend` | `partner.bsend(r_id, data)` | ✅ |
| `Par_AsBSend` | 原生 `async fn bsend` | ✅ |
| `Par_WaitAsBSendCompletion` | `.await` | ✅ |
| `Par_CheckAsBSendCompletion` | `.await` + `tokio::select!` | ✅ |
| `Par_SetSendCallback` | tokio 任务 / channel | ✅ |
| `Par_BRecv` | `partner.brecv()` | ✅ |
| `Par_CheckAsBRecvCompletion` | `.await` + `tokio::select!` | ✅ |
| `Par_SetRecvCallback` | tokio 任务 / channel | ✅ |
| `Par_GetStatus` | `partner.get_status()` → `PartnerStatus` | ✅ |
| `Par_GetTimes` | —（无逐方向耗时统计） | ❌ |
| `Par_GetStats` | `partner.get_stats()` → `(bytes_sent, bytes_recv)` | ✅ |
| `Par_GetParam` / `Par_SetParam` | `recv_timeout` / `send_timeout` 字段 | ✅ |
| `Par_GetLastError` | Rust `Result<T, Error>` | ✅ |
| `Par_ErrorText` | `Error::to_string()` | ✅ |

### rs-snap7 扩展功能（C snap7 无对应）

| 功能 | Crate |
|---|---|
| S7CommPlus（S7-1200/1500 完整性模式） | `snap7-client` |
| TLS 传输（S7CommPlus 加密） | `snap7-client` |
| UDP 传输 | `snap7-client` |
| 带最大连接数信号量的连接池 | `snap7-client` |
| 支持订阅的 OPC-UA 网关 | `snap7-opcua-gateway` |
| 类型化标签解析器（DB/M/T/C 地址语法） | `snap7-cli` |
| 支持 JSON/CSV/text 输出的 CLI | `snap7-cli` |
| 完全异步 — 每连接无阻塞线程 | 全部 crate |

---

## 安装 CLI 工具

```bash
# 主 CLI
cargo install snap7-cli --bin snap7

# 测试服务器（模拟 PLC）
cargo install snap7-cli --bin snap7-test-server

# 传感器服务器（模拟带实时 REAL 值更新的 PLC）
cargo install snap7-cli --bin snap7-sensor-server

# OPC-UA 网关及演示工具（需要 opcua feature）
cargo install snap7-cli --features opcua --bin gateway_demo
cargo install snap7-cli --features opcua --bin plc_batch_reader
cargo install snap7-cli --features opcua --bin opcua_subscriber
```

---

## 快速开始

### 连接真实 PLC

```bash
# 从 DB1 偏移 0 处读取 16 字节
snap7 -H 192.168.1.100 read --db 1 --offset 0 --size 16

# 向 DB2 偏移 4 处写入十六进制字节
snap7 -H 192.168.1.100 write --db 2 --offset 4 --data DEADBEEF

# 读取带类型的标签（DB）
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB70,332.0       # 位访问
snap7 -H 192.168.1.100 tag read DB170,REAL262     # 逗号分隔符
snap7 -H 192.168.1.100 tag read DB170.REAL262     # 点分隔符（等效）

# 读取 Merker（M）、定时器（T）、计数器（C）标签
snap7 -H 192.168.1.100 tag read MB10              # Merker 字节
snap7 -H 192.168.1.100 tag read MW20              # Merker 字
snap7 -H 192.168.1.100 tag read MD4               # Merker 双字
snap7 -H 192.168.1.100 tag read M10.3             # Merker 位（字节 10，位 3）
snap7 -H 192.168.1.100 tag read MX5.7             # Merker 位（MX 前缀）
snap7 -H 192.168.1.100 tag read T5                # 定时器 5
snap7 -H 192.168.1.100 tag read C3                # 计数器 3

# 写入带类型的标签
snap7 -H 192.168.1.100 tag write DB1,REAL0 3.14
snap7 -H 192.168.1.100 tag write DB10,DINT0 42
snap7 -H 192.168.1.100 tag write MB10 255

# 监视标签（每 500ms 轮询，仅在值变化时打印）
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 --interval-ms 500 --changes-only

# 块操作
snap7 -H 192.168.1.100 block list
snap7 -H 192.168.1.100 block numbers --type DB
snap7 -H 192.168.1.100 block info --type DB --number 1
snap7 -H 192.168.1.100 block upload --type DB --number 1 --out db1.bin

# 查询 SZL（系统状态列表）
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0

# PLC 控制
snap7 -H 192.168.1.100 plc-control status
snap7 -H 192.168.1.100 plc-control stop
snap7 -H 192.168.1.100 plc-control hotstart
snap7 -H 192.168.1.100 plc-control coldstart

# PLC 信息
snap7 -H 192.168.1.100 info order-code
snap7 -H 192.168.1.100 info cpu-info
snap7 -H 192.168.1.100 info cp-info
snap7 -H 192.168.1.100 info module-list

# 会话密码
snap7 -H 192.168.1.100 password set mypass
snap7 -H 192.168.1.100 password clear

# 运行诊断
snap7 -H 192.168.1.100 diag
```

每条命令执行完毕后，会将本次操作的往返耗时输出到 stderr：

```
exec time: 4 ms
```

### TLS 和 UDP 传输

```bash
# S7CommPlus 通过 TLS（S7-1200/1500）
snap7 -H 192.168.1.100 --tls read --db 1 --offset 0 --size 4

# 使用自定义 CA 证书
snap7 -H 192.168.1.100 --tls --tls-ca /path/to/ca.pem read --db 1 --offset 0 --size 4

# UDP 传输（ISO-on-UDP）
snap7 -H 192.168.1.100 --udp read --db 1 --offset 0 --size 4
```

### 使用本地仿真器

```bash
# 终端 1 — 在 10200 端口启动测试服务器
snap7-test-server

# 终端 2 — 读取数据
snap7 -H 127.0.0.1 -p 10200 read --db 1 --offset 0 --size 4
# → DE AD BE EF
```

### 标签地址语法

DB 标签需要在 DB 编号和类型之间使用逗号（或点）分隔符：

```
DB<n>,<类型><偏移>
DB<n>.<类型><偏移>    # 点分隔符，效果相同
DB<n>,<偏移>.<位>     # 位访问
```

Merker、定时器和计数器标签为单段格式，无需分隔符：

```
M<字节>.<位>          # Merker 位（如 M10.3）
MX<字节>.<位>         # Merker 位（MX 前缀）
MB<字节>              # Merker 字节（如 MB10）
MW<字节>              # Merker 字（如 MW20）
MD<字节>              # Merker 双字（如 MD4）
T<n>                  # 定时器（如 T5）
C<n>                  # 计数器（如 C3）
```

| 区域 | 类型 | 宽度 | 示例 |
|---|---|---|---|
| DB | `REAL` | 4 字节 | `DB1,REAL0` |
| DB | `DINT` | 4 字节 | `DB1,DINT4` |
| DB | `DWORD` | 4 字节 | `DB1,DWORD4` |
| DB | `INT` | 2 字节 | `DB1,INT8` |
| DB | `WORD` | 2 字节 | `DB1,WORD8` |
| DB | `BYTE` | 1 字节 | `DB1,BYTE10` |
| DB | 位 | 1 位 | `DB1,332.0` |
| Merker | 位 | 1 位 | `M10.3` / `MX10.3` |
| Merker | 字节 | 1 字节 | `MB10` |
| Merker | 字 | 2 字节 | `MW20` |
| Merker | 双字 | 4 字节 | `MD4` |
| 定时器 | S5Time | 2 字节 | `T5` |
| 计数器 | BCD | 2 字节 | `C3` |

### 输出格式

```bash
snap7 -H 192.168.1.100 -f json  tag read DB1,REAL0
snap7 -H 192.168.1.100 -f csv   tag read DB1,REAL0
snap7 -H 192.168.1.100 -f text  tag read DB1,REAL0   # 默认
```

---

## 作为库使用

在 `Cargo.toml` 中添加：

```toml
[dependencies]
snap7-client  = { git = "https://github.com/cool0looc/rs-snap7" }
snap7-server  = { git = "https://github.com/cool0looc/rs-snap7" }  # 可选
snap7-partner = { git = "https://github.com/cool0looc/rs-snap7" }  # 可选
```

### 异步客户端（S7Comm — S7-300/400）

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

    // 读写 DB
    let data = client.db_read(1, 0, 4).await?;
    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // Merker / 过程 I/O
    let _mk = client.mb_read(10, 4).await?;
    client.mb_write(10, &[0xFF]).await?;
    let _pi = client.eb_read(0, 2).await?;
    let _po = client.ib_read(0, 2).await?;

    // 定时器和计数器
    let _timers   = client.tm_read(0, 5).await?;
    let _counters = client.ct_read(0, 3).await?;

    // 多读 / 多写（单 PDU，自动分批）
    use snap7_client::{MultiReadItem, MultiWriteItem};
    let items = vec![MultiReadItem::db(1, 0, 4), MultiReadItem::db(2, 0, 2)];
    let _results = client.read_multi_vars(&items).await?;

    let items = vec![MultiWriteItem::db(1, 0, vec![0xAA, 0xBB])];
    client.write_multi_vars(&items).await?;

    // 指定传输类型读写任意区域
    use snap7_client::proto::s7::header::{Area, TransportSize};
    let _data = client.read_area(Area::Marker, 0, 10, 1, TransportSize::Byte).await?;
    let _data = client.read_area(Area::Timer, 0, 5, 1, TransportSize::Timer).await?;
    client.write_area(Area::Marker, 0, 10, TransportSize::Byte, &[0xFF]).await?;

    // PDU 长度
    let pdu = client.get_pdu_length().await;
    println!("协商 PDU 大小：{pdu} 字节");

    Ok(())
}
```

### S7CommPlus 客户端（S7-1200/1500 完整性模式）

```rust
use snap7_client::S7PlusClient;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr: SocketAddr = "192.168.1.100:102".parse()?;
    let client = S7PlusClient::connect(addr, Default::default()).await?;

    let data = client.db_read(1, 0, 4).await?;
    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // 多读
    use snap7_client::plus_client::DbVarSpec;
    let specs = vec![DbVarSpec { db: 1, offset: 0, length: 4 }];
    let _results = client.read_multi_vars(&specs).await?;

    // TLS 连接（S7CommPlus over TLS）
    let _client = S7PlusClient::connect_tls(
        addr, "plc.example.com", None, Default::default()
    ).await?;

    Ok(())
}
```

### PLC 控制与信息查询

```rust
// 状态
let status = client.get_plc_status().await?;  // Run | Stop | Unknown
client.plc_stop().await?;
client.plc_hot_start().await?;
client.plc_cold_start().await?;

// 标识信息
let oc = client.get_order_code().await?;    // 如 "6ES7 317-2EK14-0AB0"
let ci = client.get_cpu_info().await?;
let cp = client.get_cp_info().await?;

// 时钟
let dt = client.read_clock().await?;
client.set_clock(&dt).await?;
client.set_clock_to_now().await?;           // 同步 PLC 时钟到主机 UTC 时间

// SZL
let szl = client.read_szl(0x001C, 0).await?;
let ids = client.read_szl_list().await?;    // 所有可用 SZL ID
```

### 块操作

```rust
let list    = client.list_blocks().await?;
let numbers = client.list_blocks_of_type(0x41).await?;    // 所有 DB
let info    = client.get_ag_block_info(0x41, 1).await?;   // DB 1
let raw     = client.db_get(1).await?;

let data    = client.upload(0x41, 1).await?;               // 仅头部
let mc7     = client.full_upload(0x41, 1).await?;          // 含 MC7 代码
client.download(0x41, 1, &data).await?;
client.delete_block(0x41, 1).await?;
client.db_fill(1, 0x00).await?;
```

### 会话密码与保护

```rust
client.set_session_password("mypass").await?;
client.clear_session_password().await?;
let prot = client.get_protection().await?;
```

### 连接池

```rust
use snap7_client::{S7Pool, PoolConfig, ConnectParams};

let pool   = S7Pool::new(addr, ConnectParams::default(), PoolConfig { max_size: 4, ..Default::default() });
let client = pool.get().await?;
client.db_read(1, 0, 4).await?;
```

### 嵌入测试服务器

```rust
use snap7_server::{S7Server, ServerConfig, DataStore, CpuState};
use snap7_server::store::area;

let store = DataStore::new();
store.write_bytes(1, 0, &[1, 2, 3, 4]);
store.register_area(area::PROCESS_INPUTS, 1024);
store.set_cpu_state(CpuState::Run);

// 区域锁定 — 锁定期间写入被静默忽略
store.lock_area(area::DATA_BLOCK);
store.unlock_area(area::DATA_BLOCK);

// 事件队列
store.set_mask(0xFFFF_FFFF);
if let Some(ev) = store.pick_event() {
    println!("{}: area=0x{:02X}", ev.event, ev.area);
}

// 状态
let status = S7Server::get_status(&store);
println!("已连接客户端={} CPU 状态={:?}", status.clients_count, status.cpu_state);

// 回调
store.on_read(|info|  println!("读取  area=0x{:02X} db={}", info.area, info.db_number));
store.on_write(|info| println!("写入  area=0x{:02X} db={}", info.area, info.db_number));
store.on_event(|ev|   println!("事件：{ev}"));

let server = S7Server::bind(ServerConfig {
    bind_addr: "127.0.0.1:0".parse()?,
    max_connections: 8,
}).await?;
tokio::spawn(server.serve(store));
```

### S7 Partner（BSend/BRecv）

```rust
use snap7_partner::S7Partner;
use std::net::SocketAddr;

// 主动 Partner — 连接到远端
let active = S7Partner::connect("192.168.1.100:102".parse()?).await?;
active.bsend(0x0000_0001, b"hello").await?;

// 被动 Partner — 等待远端连接
let passive = S7Partner::listen("0.0.0.0:102".parse()?).await?;
let (r_id, data) = passive.brecv().await?;
println!("R_ID={r_id:#010x} data={data:?}");

// 先绑定再接受（可提前获取端口号）
let (listener, partner) = S7Partner::bind("127.0.0.1:0".parse()?).await?;
let _port = listener.local_addr()?.port();
S7Partner::accept(&partner, &listener).await?;
let (r_id, data) = partner.brecv().await?;
```

### UDP 传输

```rust
use snap7_client::{S7Client, ConnectParams, UdpTransport};

let client = S7Client::<UdpTransport>::connect_udp(addr, ConnectParams::default()).await?;
let data = client.db_read(1, 0, 4).await?;
```

---

## OPC-UA 网关

`snap7-opcua-gateway` crate（及 `snap7 serve` 命令）将 PLC 桥接至 OPC-UA 客户端，支持完整订阅功能。

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

OPC-UA 客户端订阅 `ns=2;s=Temperature` 等节点后，将按轮询间隔接收通知。Python/Node.js 订阅示例参见 [OPC-UA_SUBSCRIPTIONS.md](crates/snap7-opcua-gateway/OPC-UA_SUBSCRIPTIONS.md)。

---

## 从源码构建

```bash
git clone https://github.com/cool0looc/rs-snap7
cd rs-snap7

# 构建全部
cargo build --release

# 带 OPC-UA 网关支持构建
cargo build --release --features opcua -p snap7-cli

# 运行测试
cargo test --workspace

# 运行基准测试
cargo bench -p snap7-bench
```

---

## 许可证

MIT — 参见 [LICENSE](LICENSE)。
