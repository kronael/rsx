# DXS / WAL / CMP Architecture

Transport and persistence layer for all inter-process
communication.

## Three Paths

```
Hot Path (CMP/UDP)     Cold Path (WAL/TCP)     Archive
+----------------+     +------------------+    +----------+
| CmpSender      |     | WalWriter        |    | Recorder |
| CmpReceiver    |     | WalReader        |    | (daily   |
| flow control   |     | DxsReplayService |    |  files)  |
| heartbeat/NACK |     | DxsConsumer      |    +----------+
+----------------+     +------------------+
```

### CMP (C Message Protocol)

- UDP datagrams between processes
- Fixed header: record_type(u16) + len(u16) + seq
- Flow control: window-based, NACK on gap
- Heartbeat: detect dead producers
- Configurable via env vars (CmpConfig)

### WAL (Write-Ahead Log)

- 16B header + repr(C, align(64)) payload
- Disk format = wire format = memory format
- Flush: 10ms or buffer full
- Rotate: 64MB files
- Retain: 10min (hot), infinite (archive)
- CRC32 validation on read

### DXS (Data Exchange Streaming)

- Each producer IS the replay server
- Consumers connect directly (no broker)
- TCP replay from any seq
- TLS support
- Tip tracking with persistence
- Reconnect with exponential backoff

## Record Types

| Type | Name | Direction |
|------|------|-----------|
| 0x01 | OrderRequest | GW -> Risk -> ME |
| 0x02 | OrderInserted | ME -> Risk, Mktdata |
| 0x03 | OrderCancelled | ME -> Risk, Mktdata |
| 0x04 | Fill | ME -> Risk, Mktdata |
| 0x05 | OrderDone | ME -> Risk -> GW |
| 0x06 | CancelRequest | GW -> Risk -> ME |
| 0x07 | ConfigApplied | ME -> Risk, Mktdata |
| 0x08 | OrderFailed | Risk -> GW |
| 0x09 | OrderAccepted | ME -> WAL (dedup) |
| 0x0A | MarkPrice | Mark -> WAL, CMP to Risk |
| 0x0B | BBO | ME -> Risk (CMP only) |
| 0x10 | CaughtUp | DXS -> consumer |

## WAL Dump Tool

`rsx-wal-dump` binary for debugging WAL files.
Reads and prints records with seq, type, payload.

## Specs

- [specs/v1/DXS.md](../specs/v1/DXS.md)
- [specs/v1/WAL.md](../specs/v1/WAL.md)
- [specs/v1/CMP.md](../specs/v1/CMP.md)
