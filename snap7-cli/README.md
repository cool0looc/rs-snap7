# snap7-cli

Command-line tool for communicating with Siemens S7 PLCs. Pure Rust — no FFI, no native C dependency. Read/write data blocks, watch tags, query SZL, upload blocks, and run an OPC-UA gateway — all from the terminal.

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
snap7 -H 192.168.1.100 read --db 1 --offset 0 --size 4 --area merker
```

### write — raw DB write (hex bytes)

```bash
snap7 -H 192.168.1.100 write --db 2 --offset 4 --data DEADBEEF
```

### tag — typed read/write

Supports DB, Merker (M), Timer (T), and Counter (C) tags:

```bash
# DB tags
snap7 -H 192.168.1.100 tag read DB1,REAL0
snap7 -H 192.168.1.100 tag read DB170.REAL262    # dot separator
snap7 -H 192.168.1.100 tag read DB70,332.0       # bit access
snap7 -H 192.168.1.100 tag write DB1,REAL0 3.14
snap7 -H 192.168.1.100 tag write DB10,DINT0 42

# Merker tags
snap7 -H 192.168.1.100 tag read MB10             # byte
snap7 -H 192.168.1.100 tag read MW20             # word
snap7 -H 192.168.1.100 tag read MD4              # dword
snap7 -H 192.168.1.100 tag read M10.3            # bit (byte 10, bit 3)
snap7 -H 192.168.1.100 tag read MX5.7            # bit (MX prefix)
snap7 -H 192.168.1.100 tag write MB10 255

# Timer and Counter
snap7 -H 192.168.1.100 tag read T5               # Timer 5
snap7 -H 192.168.1.100 tag read C3               # Counter 3
```

### watch — poll a DB region

```bash
snap7 -H 192.168.1.100 watch --db 1 --offset 0 --size 4 \
    --interval-ms 500 --changes-only
```

### block — block operations

```bash
# List all blocks grouped by type
snap7 -H 192.168.1.100 block list

# List all block numbers of a given type
snap7 -H 192.168.1.100 block numbers --type DB
snap7 -H 192.168.1.100 block numbers --type OB

# Show detailed info for a block
snap7 -H 192.168.1.100 block info --type DB --number 1

# Upload a block to file
snap7 -H 192.168.1.100 block upload --type DB --number 1 --out db1.bin
```

### szl — query system status list

```bash
snap7 -H 192.168.1.100 szl --id 0x0011 --index 0   # order code
snap7 -H 192.168.1.100 szl --id 0x001C --index 0   # CPU info
snap7 -H 192.168.1.100 szl --id 0x0424 --index 0   # CPU status
```

### plc-control — PLC state management

```bash
snap7 -H 192.168.1.100 plc-control status      # RUN / STOP (via SZL 0x0424)
snap7 -H 192.168.1.100 plc-control stop
snap7 -H 192.168.1.100 plc-control hotstart    # warm restart
snap7 -H 192.168.1.100 plc-control coldstart   # cold restart
```

### info — PLC information

```bash
snap7 -H 192.168.1.100 info order-code         # e.g. "6ES7 317-2EK14-0AB0"
snap7 -H 192.168.1.100 info cpu-info           # module type, serial, AS name
snap7 -H 192.168.1.100 info cp-info            # PDU size, connections, baud rates
snap7 -H 192.168.1.100 info module-list        # installed modules
```

### password — session password

```bash
snap7 -H 192.168.1.100 password set mypass
snap7 -H 192.168.1.100 password clear
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

DB tags use a comma or dot separator:

```
DB<n>,<type><byte-offset>
DB<n>.<type><byte-offset>   # dot separator, same result
DB<n>,<byte-offset>.<bit>   # bit access
```

Merker, Timer, Counter tags are single-part — no separator:

```
M<byte>.<bit>    MX<byte>.<bit>    MB<byte>    MW<byte>    MD<byte>
T<n>    C<n>
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

MIT — see [LICENSE](../LICENSE).
