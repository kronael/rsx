# rsx-dxs

WAL writer/reader, DXS replay server/client, and CMP
(C Message Protocol) UDP transport. Library crate used by
all binary processes.

## What It Provides

- `WalWriter` -- append-only WAL with fsync, rotation, GC
- `WalReader` -- sequential read with CRC validation
- `CmpSender` / `CmpReceiver` -- UDP transport with NACK
- `DxsReplayService` -- TCP replay server (historical + live)
- `DxsConsumer` -- TCP replay client with tip persistence
- All `#[repr(C, align(64))]` record structs and `CmpRecord`
  trait

## Public API

```rust
use rsx_dxs::WalWriter;
use rsx_dxs::WalReader;
use rsx_dxs::CmpSender;
use rsx_dxs::CmpReceiver;
use rsx_dxs::DxsReplayService;
use rsx_dxs::DxsConsumer;
use rsx_dxs::records::FillRecord;
```

Used by: rsx-matching, rsx-risk, rsx-gateway, rsx-marketdata,
rsx-mark, rsx-recorder, rsx-cli.

## Environment Variables

| Env Var | Purpose |
|---------|---------|
| `RSX_WAL_DIR` | WAL directory |
| `RSX_WAL_ARCHIVE_DIR` | Archive directory |
| `RSX_WAL_MAX_FILE_SIZE` | Max file before rotation (64MB) |
| `RSX_WAL_RETENTION_NS` | Retention window (10min) |
| `RSX_CMP_REORDER_BUF_LIMIT` | Max reorder buffer (512) |
| `RSX_CMP_HEARTBEAT_INTERVAL_MS` | Heartbeat interval (10ms) |
| `RSX_REPL_TLS` | Enable TLS for replay |
| `RSX_REPL_CERT_PATH` | TLS certificate path |
| `RSX_REPL_KEY_PATH` | TLS private key path |

## Building

```
cargo check -p rsx-dxs
cargo build -p rsx-dxs
```

## Testing

```
cargo test -p rsx-dxs
```

7 test files: archive, client, CMP, header, records, TLS, WAL.
All tests non-flaky (unique temp dirs, ephemeral ports).
Benchmarks: `wal_bench`, `encode_bench`.
See `specs/v1/TESTING-DXS.md`.

## Dependencies

- `rsx-types` -- Price, Qty, Side

## Gotchas

- WAL bytes = disk bytes = wire bytes = memory bytes. No
  serialization step. Same `#[repr(C)]` structs everywhere.
- WAL files have no header or index. Sequential read only.
  Filenames encode seq range for O(1) file selection.
- WalWriter flush is caller-driven (not background thread).
  If the caller doesn't call flush, nothing hits disk.
- CMP reorder buffer is a BTreeMap. Under sustained packet
  loss, it grows up to `RSX_CMP_REORDER_BUF_LIMIT` entries.
- TLS is optional for replay connections. CMP/UDP has no
  encryption (same-machine only).

## See Also

- [ARCHITECTURE.md](ARCHITECTURE.md) -- WAL format, CMP
  protocol, replay protocol, record types, performance
- `specs/v1/DXS.md`, `specs/v1/WAL.md`, `specs/v1/CMP.md`
