# rs-snap7

纯 Rust 异步实现的西门子 S7 协议栈。通过 ISO-on-TCP 与 S7-300/400/1200/1500 系列 PLC 通信，无需任何 C 原生依赖。

> **状态：** 功能完整，1.0 前版本，API 可能变动。

## 工作区 crate

| Crate | 说明 |
|---|---|
| [`snap7-client`](crates/snap7-client) | 异步 PLC 客户端（`S7Client`、`S7PlusClient`、连接池、TLS）—— 包含协议层 |
| [`snap7-server`](crates/snap7-server) | 进程内 PLC 仿真器，用于测试 |
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

# 监视标签（每 500ms 轮询，仅在值变化时打印）
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 --interval-ms 500 --changes-only

# 上传块
snap7 -H 192.168.1.100 block upload --type OB --number 1 --out ob1.bin

# 列出块
snap7 -H 192.168.1.100 block list

# 查询 SZL（系统状态列表）
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0

# 运行诊断
snap7 -H 192.168.1.100 diag
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

### 异步客户端

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

    // 多读（单 PDU）
    use snap7_client::MultiReadItem;
    let items = vec![
        MultiReadItem::db(1, 0, 4),
        MultiReadItem::db(2, 0, 2),
    ];
    let results = client.multi_read(items).await?;

    Ok(())
}
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
// addr 现已可接受 S7 连接
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
