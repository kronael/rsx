# rsx-cli

CLI tools for RSX exchange operations.

## Commands

### wal-dump

Dump WAL records for a given stream.

```
cargo run -p rsx-cli -- wal-dump <stream_id> <wal_dir> [from_seq]
```

Example:
```
cargo run -p rsx-cli -- wal-dump 1 ./tmp/wal 0
```

Output:
```
seq=1        type=ORDER_ACCEPTED    len=64   crc=0x1a2b3c4d
seq=2        type=FILL              len=96   crc=0x5e6f7a8b
seq=3        type=ORDER_DONE        len=80   crc=0x9c0d1e2f
total: 3 records
```

## Building

```
cargo build -p rsx-cli
```

## Dependencies

- `clap` -- CLI argument parsing
- `rsx-dxs` -- WAL reader and record type constants

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- command internals
