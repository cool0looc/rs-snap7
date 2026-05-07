# snap7-opcua-gateway

OPC-UA server that bridges Siemens S7 PLCs to OPC-UA clients. Pure Rust — no FFI, no native C dependency. Polls PLC tags at a configurable interval and exposes them as OPC-UA variable nodes with subscription support.

Part of the [rs-snap7](https://github.com/cool0looc/rs-snap7) workspace.

## Features

| Capability | Status |
|---|---|
| **S7 tag polling** — any tag address, configurable interval | ✅ |
| **OPC-UA variable nodes** — `ns=2;s=<name>` | ✅ |
| **Subscriptions** — OPC-UA clients get change notifications | ✅ |
| **Writable tags** — write-through from OPC-UA to PLC | ✅ |
| **TOML configuration file** | ✅ |
| **Async, non-blocking** — tokio-based polling loop | ✅ |
| **Pure Rust, zero native dependencies** | ✅ |

## Add to your project

```toml
[dependencies]
snap7-opcua-gateway = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Configuration

```toml
# gateway.toml
plc_addr = "192.168.1.100:102"
opc_endpoint = "opc.tcp://0.0.0.0:4840"
poll_interval_ms = 500         # default: 1000

[[tags]]
name = "Temperature"           # OPC-UA node name (ns=2;s=Temperature)
tag = "DB1,REAL0"
writable = false

[[tags]]
name = "Setpoint"
tag = "DB2,REAL0"
writable = true
```

## Programmatic usage

```rust
use snap7_opcua_gateway::{Gateway, GatewayConfig};

let config: GatewayConfig = toml::from_str(include_str!("gateway.toml"))?;
let gateway = Gateway::new(config).await?;
gateway.run().await?;
```

## OPC-UA node IDs

Tags are exposed as `ns=2;s=<name>`. Subscribe or read with any OPC-UA client:

```python
# Python example (asyncua)
from asyncua import Client
async with Client("opc.tcp://localhost:4840") as client:
    node = client.get_node("ns=2;s=Temperature")
    print(await node.read_value())
```

See [OPC-UA_SUBSCRIPTIONS.md](OPC-UA_SUBSCRIPTIONS.md) for Python and Node.js subscription examples.

## Architecture

```
PLC ──S7Comm──► PlcPoller ──► TagRegistry ──► OPC-UA address space
                  (async)        (shared)       (async-opcua server)
```

- `PlcPoller` — reads tags from the PLC on each tick, updates `TagRegistry`
- `TagRegistry` — stores current tag values; drives OPC-UA change notifications
- `Gateway` — wires the OPC-UA server, poller, and registry together

## License

MIT — see [LICENSE](../../LICENSE).
