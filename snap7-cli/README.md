# snap7-cli

Command-line tool for communicating with Siemens S7 PLCs. Pure Rust — no FFI, no native C dependency. Read/write data blocks, watch tags, upload blocks, query SZL, and run an OPC-UA gateway — all from the terminal.

Part of the [rs-snap7](https://github.com/cool0looc/rs-snap7) workspace.

## Install

```bash
# Main CLI
cargo install snap7-cli --bin snap7

# Simulated PLC (for local testing)
cargo install snap7-cli --bin snap7-test-server

# Sensor simulator (live-updating REAL values)
cargo install snap7-cli --bin snap7-sensor-server

# OPC-UA tools (requires opcua feature)
cargo install snap7-cli --features opcua --bin gateway_demo
cargo install snap7-cli --features opcua --bin plc_batch_reader
cargo install snap7-cli --features opcua --bin opcua_subscriber
```

## Global flags

```
snap7 [FLAGS] <SUBCOMMAND>

  -H, --host <HOST>           PLC IP address
  -p, --port <PORT>           TCP port [default: 102]
  -r, --rack <RACK>           Rack number [default: 0]
  -s, --slot <SLOT>           Slot number [default: 1]
  -f, --format <FORMAT>       Output format: text|json|csv [default: text]
  -t, --timeout-secs <SECS>   Connect timeout [default: 5]
      --tls                   Use TLS (S7CommPlus encrypted mode)
      --tls-ca <PATH>         PEM CA cert for TLS verification
      --udp                   Use UDP transport
```

## Subcommands

### read — raw DB read

```bash
snap7 -H 192.168.1.100 read --db 1 --offset 0 --size 16
```

### write — raw DB write (hex bytes)

```bash
snap7 -H 192.168.1.100 write --db 2 --offset 4 --data DEADBEEF
```

### tag — typed read/write

```bash
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB70,332.0     # bit access
snap7 -H 192.168.1.100 tag write DB1,REAL0 3.14
```

### watch — poll a DB region

```bash
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 \
    --interval-ms 500 --changes-only
```

### block — upload or list blocks

```bash
snap7 -H 192.168.1.100 block upload --type OB --number 1 --out ob1.bin
snap7 -H 192.168.1.100 block list
```

### szl — query system status list

```bash
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0
```

### diag — connection diagnostics

```bash
snap7 -H 192.168.1.100 diag
```

### serve — OPC-UA gateway (requires `opcua` feature)

```bash
snap7 -H 192.168.1.100 serve --config gateway.toml
```

See [snap7-opcua-gateway](../crates/snap7-opcua-gateway/README.md) for config format.

## Output formats

All subcommands honour `-f`:

```bash
snap7 -H 192.168.1.100 -f json tag read DB1,REAL0
snap7 -H 192.168.1.100 -f csv  tag read DB1,REAL0
snap7 -H 192.168.1.100 -f text tag read DB1,REAL0   # default
```

## Local testing with the simulator

```bash
# Terminal 1 — simulated PLC on port 10200
snap7-test-server

# Terminal 2
snap7 -H 127.0.0.1 -p 10200 read --db 1 --offset 0 --size 4
# → DE AD BE EF
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

MIT — see [LICENSE](../LICENSE).
