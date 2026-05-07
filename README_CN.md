# rs-snap7

纯 Rust 异步实现的西门子 S7 协议栈。通过 ISO-on-TCP 与 S7-300/400/1200/1500 系列 PLC 通信，无需任何 C 原生依赖。

> **状态：** v0.1.3 — 功能完整，1.0 前版本，API 可能变动。

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
| **块操作** — 列表、信息、上传、下载、删除、填充 | ✅ |
| **会话密码** — 设置、清除、读取保护级别 | ✅ |
| **SZL 查询** — 系统状态列表 | ✅ |
| **PLC 时钟读取** | ✅ |
| **拷贝 RAM → ROM、压缩内存** | ✅ |
| **CLI** — 类型化标签、多格式输出（text/json/csv） | ✅ |
| **OPC-UA 网关**，支持订阅 | ✅ |
| **进程内 PLC 模拟器**，含数据存储、回调和 CPU 状态 | ✅ |

## 工作区 crate

| Crate | 说明 |
|---|---|
| [`snap7-client`](crates/snap7-client) | 异步 PLC 客户端（`S7Client`、`S7PlusClient`、连接池、TLS、UDP）—— 包含协议层 |
| [`snap7-server`](crates/snap7-server) | 进程内 PLC 仿真器，含数据存储、回调和 CPU 状态 |
| [`snap7-opcua-gateway`](crates/snap7-opcua-gateway) | 支持订阅的 OPC-UA 网关 |
| [`snap7-cli`](snap7-cli) | CLI 二进制（`snap7`）及辅助服务器 |

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

# 读取带类型的标签
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB70,332.0       # 位访问

# 写入带类型的标签
snap7 -H 192.168.1.100 tag write DB1,REAL0 3.14
snap7 -H 192.168.1.100 tag write DB10,DINT0 42

# 监视标签（每 500ms 轮询，仅在值变化时打印）
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 --interval-ms 500 --changes-only

# 块操作
snap7 -H 192.168.1.100 block list
snap7 -H 192.168.1.100 block info --type OB --number 1
snap7 -H 192.168.1.100 block upload --type OB --number 1 --out ob1.bin

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

# 会话密码
snap7 -H 192.168.1.100 password set mypass
snap7 -H 192.168.1.100 password clear

# 运行诊断
snap7 -H 192.168.1.100 diag
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

```
DB<n>,<类型><偏移>
DB<n>,<偏移>.<位>
```

| 类型 | 长度 | 示例 |
|---|---|---|
| `REAL` | 4 字节 | `DB1,REAL0` |
| `DINT` | 4 字节 | `DB1,DINT4` |
| `DWORD` | 4 字节 | `DB1,DWORD4` |
| `INT` | 2 字节 | `DB1,INT8` |
| `WORD` | 2 字节 | `DB1,WORD8` |
| `BYTE` | 1 字节 | `DB1,BYTE10` |
| 位 | 1 位 | `DB1,332.0` |

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
snap7-client = { git = "https://github.com/cool0looc/rs-snap7" }
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

    // 从 DB1 偏移 0 处读取 4 字节
    let data = client.read_db(1, 0, 4).await?;
    println!("{data:?}");

    // 写入
    client.write_db(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // 多读（单 PDU，自动分批）
    use snap7_client::MultiReadItem;
    let items = vec![
        MultiReadItem::db(1, 0, 4),
        MultiReadItem::db(2, 0, 2),
    ];
    let results = client.read_multi_vars(&items).await?;

    // 多写（单 PDU，自动分批）
    use snap7_client::MultiWriteItem;
    let items = vec![
        MultiWriteItem::db(1, 0, vec![0xAA, 0xBB]),
        MultiWriteItem::db(2, 10, vec![0x01, 0x02]),
    ];
    client.write_multi_vars(&items).await?;

    // 绝对区域读写（不限于 DB，可操作任意区域）
    use snap7_client::proto::s7::header::Area;
    let data = client.ab_read(Area::Merker, 0, 0, 4).await?;
    client.ab_write(Area::ProcessOutputs, 0, 0, &[0x00]).await?;

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
    println!("{data:?}");

    client.db_write(1, 0, &[0xDE, 0xAD, 0xBE, 0xEF]).await?;

    // 多读
    use snap7_client::plus_client::DbVarSpec;
    let specs = vec![
        DbVarSpec { db: 1, offset: 0, length: 4 },
        DbVarSpec { db: 2, offset: 0, length: 2 },
    ];
    let results = client.read_multi_vars(&specs).await?;

    // TLS 连接（S7CommPlus over TLS）
    let client = S7PlusClient::connect_tls(
        addr, "plc.example.com", None, Default::default()
    ).await?;

    Ok(())
}
```

### PLC 控制与信息查询

```rust
// 读取 PLC 状态（RUN / STOP）
let status = client.get_plc_status().await?;

// 控制 PLC
client.plc_stop().await?;
client.plc_hot_start().await?;
client.plc_cold_start().await?;

// 读取订货号（如 "6ES7 317-2EK14-0AB0"）
let oc = client.get_order_code().await?;
println!("Order code: {}", oc.code);

// 读取详细 CPU 信息
let ci = client.get_cpu_info().await?;
println!("Module: {}", ci.module_type);
println!("Serial: {}", ci.serial_number);

// 读取 CP 信息（最大 PDU 大小、连接数、波特率）
let cp = client.get_cp_info().await?;

// 读取模块列表
let modules = client.read_module_list().await?;
```

### 块操作

```rust
// 列出所有块
let list = client.list_blocks().await?;
for entry in &list.entries {
    println!("类型 0x{:04X}: {} 个块", entry.block_type, entry.count);
}

// 获取块详细信息
let info = client.get_ag_block_info(0x41, 1).await?; // DB 1
println!("大小: {} 字节", info.size);
println!("作者: {}", info.author);

// 上传块（Diagra 格式）
let data = client.upload(0x41, 1).await?; // DB 1
if let Some(bd) = snap7_client::BlockData::from_bytes(&data) {
    println!("已上传块 {} 字节", bd.total_length);
}

// 下载块
client.download(0x41, 1, &data).await?;

// 删除块
client.delete_block(0x41, 1).await?;

// 用常量值填充 DB
client.db_fill(1, 0x00).await?;
```

### 会话密码与保护

```rust
// 设置会话密码
client.set_session_password("mypass").await?;

// 清除会话密码
client.clear_session_password().await?;

// 读取保护级别
let protection = client.get_protection().await?;
println!("密码已设置: {}", protection.password_set);
println!("级别: {}", protection.level);
```

### 其他客户端操作

```rust
// 读取 PLC 时钟
let dt = client.read_clock().await?;

// 拷贝 RAM 到 ROM（断电保持）
client.copy_ram_to_rom().await?;

// 压缩 PLC 工作内存（需处于 STOP 模式）
client.compress().await?;

// 运行时参数调整
let timeout = client.request_timeout();
client.set_request_timeout(std::time::Duration::from_secs(3)).await;
```

### 连接池

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

### 嵌入测试服务器

```rust
use snap7_server::{S7Server, ServerConfig, DataStore, CpuState};
use std::net::SocketAddr;

let store = DataStore::new();
store.write_bytes(1, 0, &[1, 2, 3, 4]);

// 区域注册（非 DB 读取需要）
store.register_area(0x81, 1024);   // 过程输入
store.register_area(0x82, 1024);   // 过程输出

// CPU 状态追踪
store.set_cpu_state(CpuState::Run);

// 事件回调
store.on_read(|info| {
    println!("读取  区域=0x{:02X} DB={}", info.area, info.db_number);
});
store.on_write(|info| {
    println!("写入  区域=0x{:02X} DB={}", info.area, info.db_number);
});

let cfg = ServerConfig {
    bind_addr: "127.0.0.1:0".parse()?,
    max_connections: 8,
};
let server = S7Server::bind(cfg).await?;
let addr = server.local_addr()?;

tokio::spawn(server.serve(store));
// addr 现已可接受 S7 连接
```

### UDP 传输

```rust
use snap7_client::{S7Client, ConnectParams, UdpTransport};
use std::net::SocketAddr;

let addr: SocketAddr = "192.168.1.100:102".parse()?;
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
