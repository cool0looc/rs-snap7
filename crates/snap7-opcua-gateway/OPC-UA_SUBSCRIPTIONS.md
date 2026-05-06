# OPC-UA Subscriptions Support

The snap7-opcua-gateway fully supports OPC-UA subscriptions, allowing clients to receive real-time notifications when PLC data changes.

## How It Works

```
┌─────────────────┐    ┌──────────────────┐    ┌───────────────┐
│  S7 Sensor      │    │  OPC-UA Gateway  │    │  OPC-UA       │
│  Server         │───▶│  PlcPoller       │───▶│  Client       │
│  (1000ms)       │    │  (500ms)         │    │  Subscription │
└─────────────────┘    └──────────────────┘    └───────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │  OPC-UA Address  │
                     │  Space Variables │
                     └──────────────────┘
```

## Implementation Details

### 1. Variable Registration (server.rs)

Variables are registered with proper access levels:

```rust
VariableBuilder::new(space, &node_id)
    .data_type(DataTypeId::Double)
    .access_level(AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE)
    .writable()
    .value(initial_value.clone())
    .insert(&objects_folder)?;
```

### 2. Data Update with Notification (poller.rs)

When PLC data is read, values are updated in the address space and subscriptions are notified:

```rust
// Update OPC-UA variable value
if let NodeType::Variable(var) = node {
    var.as_mut().set_value(variant.clone());
    
    // Notify subscribed clients
    let entry = MonitoredItemEntry {
        node_id: node_id.clone(),
        attribute_id: AttributeId::Value.into(),
        value: DataValue::new_at(variant, now),
    };
    server_handle
        .subscriptions()
        .notify_data_change([entry].into_iter());
}
```

### 3. Client Subscription Example

Clients can use standard OPC-UA subscription APIs:

**Python (asyncua):**
```python
from asyncua import Client
from asyncua.common.subscription import DataChangeNotificationHandler

class MyHandler(DataChangeNotificationHandler):
    def datachange_notification(self, node, val, data):
        print(f"{node}: {val}")

async def main():
    client = Client("opc.tcp://127.0.0.1:4840")
    await client.connect()
    
    handler = MyHandler()
    subscription = await client.create_subscription(500, handler)
    
    # Subscribe to variables
    temp_node = client.get_node("ns=2;s=Temperature")
    await subscription.subscribe_data_change(temp_node)
    
    await asyncio.sleep(10)  # Wait for notifications
    
    await subscription.delete()
    await client.disconnect()

asyncio.run(main())
```

**Node.js (node-opcua):**
```javascript
const { OPCUAClient, DataChangeFilter } = require("node-opcua");

const client = OPCUAClient.create({
    endpointMustExist: false
});

await client.connect("opc.tcp://127.0.0.1:4840");

const session = await client.createSession();

// Subscribe to variables
const subscription = await session.createSubscription({
    publishingInterval: 500,
    maxNotificationsPerPublish: 100
});

const monitoredItem = await subscription.monitor(
    "ns=2;s=Temperature",
    DataChangeFilter.statusValueTimestamp,
    { queueSize: 10 }
);

monitoredItem.on("changed", (dataValue) => {
    console.log(`Temperature: ${dataValue.value.value}`);
});

// Keep subscription alive
await new Promise(resolve => setTimeout(resolve, 10000));

await subscription.terminate();
await session.close();
await client.disconnect();
```

## Configuration

The gateway automatically handles subscription infrastructure. No special configuration is needed for subscriptions.

### Gateway Config Example

```toml
plc_addr = "127.0.0.1:10200"
opc_endpoint = "opc.tcp://0.0.0.0:4840"
poll_interval_ms = 500

[[tags]]
name = "Temperature"
tag = "DB1,REAL0"
writable = false

[[tags]]
name = "Humidity"
tag = "DB2,REAL0"
writable = false

[[tags]]
name = "Pressure"
tag = "DB3,REAL0"
writable = false
```

The OPC-UA namespace URI is fixed at `urn:snap7-opcua-gateway:tags` (namespace index 2). Variables are addressed as `ns=2;s=<name>`.

## Testing

Run the subscription test:

```bash
# Start sensor server
cargo run --release -p snap7-cli --bin snap7-sensor-server &

# Start gateway
cargo run --release -p snap7-cli --bin snap7 --features opcua -- serve -c scripts/gateway-config.toml &

# Run subscription test
python3 scripts/test_opcua_subscription.py
```

## Test Results

✅ **Verified Working (2024)**

```
Total notifications received: 30 (in 10 seconds)

Data Changes Detected:
  - Temperature: 25.71 → 24.68°C
  - Humidity: 56.72 → 58.06%
  - Pressure: 102.89 → 103.16 kPa

Notification Latency: ~500ms (matches publishing interval)
```

## Features

- ✅ Real-time data change notifications
- ✅ Configurable publishing interval (default: 500ms)
- ✅ Read/Write variable support
- ✅ Multiple concurrent subscriptions per session
- ✅ Multiple sessions support
- ✅ Automatic data type conversion (S7 REAL → OPC-UA Double)

## Performance

| Metric | Value |
|--------|-------|
| Publishing Interval | 500ms (configurable) |
| Notification Latency | ~500ms |
| Max Subscriptions | Limited by server resources |
| Data Types | REAL, INT, DINT, BYTE, BOOL |
