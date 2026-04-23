# rsx-dxs Architecture

WAL writer/reader, DXS replay server, DXS consumer client,
and CMP (C Message Protocol) sender/receiver.
See `specs/1/10-dxs.md`, `specs/1/48-wal.md`, `specs/1/4-cmp.md`.

## Module Layout

| File | Purpose |
|------|---------|
| `header.rs` | `WalHeader` -- 16-byte manual encode/decode |
| `records.rs` | All `#[repr(C, align(64))]` record structs, `CmpRecord` trait, `WalRecord` enum |
| `encode_utils.rs` | CRC32, `as_bytes()`, encode/decode helpers per record type |
| `wal.rs` | `WalWriter`, `WalReader`, `RawWalRecord`, file listing/rotation |
| `server.rs` | `DxsReplayService` -- TCP replay server with TLS |
| `client.rs` | `DxsConsumer` -- TCP replay client with tip persistence |
| `cmp.rs` | `CmpSender`, `CmpReceiver` -- UDP transport with NACK |
| `config.rs` | `DxsConfig`, `RecorderConfig`, `CmpConfig`, `TlsConfig` |

## Transport Paths

- **CMP/UDP** (hot path): Aeron-inspired protocol with
  NACK-based recovery. `CmpSender` sends sequenced records,
  `CmpReceiver` detects gaps and sends NAK. Heartbeats every
  10ms. Reorder buffer (BTreeMap) handles out-of-order packets.
- **WAL/TCP** (cold path): `DxsReplayService` serves
  historical replay + live tail over TCP (optionally TLS).
  `DxsConsumer` connects, sends `ReplayRequest`, receives
  records, persists tip every 10ms.

## Record Types

| Const | Type |
|-------|------|
| `RECORD_FILL` (0) | Fill |
| `RECORD_BBO` (1) | Best bid/ask |
| `RECORD_ORDER_INSERTED` (2) | Order resting |
| `RECORD_ORDER_CANCELLED` (3) | Order cancelled |
| `RECORD_ORDER_DONE` (4) | Order completed |
| `RECORD_CONFIG_APPLIED` (5) | Config change |
| `RECORD_CAUGHT_UP` (6) | Replay caught up |
| `RECORD_ORDER_ACCEPTED` (7) | Dedup acceptance |
| `RECORD_MARK_PRICE` (8) | Mark price |
| `RECORD_ORDER_FAILED` (12) | Pre-trade reject |
| `RECORD_STATUS_MESSAGE` (0x10) | CMP flow control |
| `RECORD_NAK` (0x11) | CMP gap NACK |
| `RECORD_HEARTBEAT` (0x12) | CMP heartbeat |

All data records implement `CmpRecord` trait (seq get/set,
record_type).

## WAL Record Format

Every record: 16-byte header + fixed-size payload.

```
struct WalHeader {       // 16 bytes
    record_type: u16,    // message type enum
    len: u16,            // payload length (<= 64KB)
    crc32: u32,          // CRC32 of payload
    _reserved: [u8; 8],
}
```

Payload: `#[repr(C, align(64))]`, little-endian. Data payloads
implement CmpRecord trait with `seq: u64` as first 8 bytes.

## WalWriter Internals

**Append:** assigns monotonic seq, serializes to buf. O(1)
memcpy, <200ns.

**Flush:** every 10ms, write buf to file + fsync. Called by
producer's main loop (not a background thread).

**Rotation:** on flush, if file_size > 64MB: rename active
file with seq range, open new file, run GC.

**GC:** scan dir, parse filenames, delete files older than
retention (mtime-based). Runs on rotation.

**Backpressure:** buf > 2x max_file_size -> append blocks.

## File Layout

```
wal/{stream_id}/{stream_id}_{first_seq}_{last_seq}.wal
wal/{stream_id}/{stream_id}_active.wal  (current)
```

No file header, no index. Sequential read only. Filenames
encode seq range for O(1) file selection.

## DxsReplayService Protocol

```
1. Consumer connects (optional TLS)
2. Consumer sends ReplayRequest {stream_id, from_seq}
3. Server opens WalReader at from_seq
4. Server streams raw WAL bytes (header + payload)
5. On caught-up: sends RECORD_CAUGHT_UP {live_seq}
6. Transitions to live broadcast (notify on flush)
```

CaughtUp: `live_seq` is inclusive. Consumer resumes at
`live_seq + 1`. Per-symbol stream.

## Idempotent Replay

Consumer dedup by seq. Risk: fill with
`seq <= tips[symbol_id]` is a no-op.

## Edge Cases

- Crash mid-rotation: active file recovered by CRC
- Partial record at EOF: detected, truncated
- CRC mismatch: conservative truncation at first bad record
- Unknown record type: returned as raw, consumer skips
- Gap in seq: consumer must use archive fallback
- Concurrent readers: filesystem provides read safety

## CMP/UDP Flow Control (Aeron Model)

- Receiver sends StatusMessage every 10ms:
  `{consumption_seq, receiver_window}`
- Sender tracks `consumption_seq + receiver_window`
- Sender stalls if no room (returns false)

Gap detection: receiver expects sequential seq. CmpHeartbeat
tells receiver sender's highest_seq. Gap -> Nak immediately.
Sender reads missing records from WAL, resends. Nak
suppression: 1ms coalesce window.

## Wire Format Invariant

```
WAL bytes = disk bytes = wire bytes = memory bytes
```

Same `#[repr(C, align(64))]` structs everywhere. No
serialization step. CRC32 in header covers payload.

## Consumers

| Consumer | Source | Purpose |
|----------|--------|---------|
| Risk | ME WAL | Fill ingestion, position update |
| Risk | Mark WAL | Mark price feed |
| Marketdata | ME WAL | Shadow book bootstrap |
| Recorder | ME WAL | Daily archival |

## Connection Topology

```
Gateway --[CMP/UDP]--> Risk --[CMP/UDP]--> ME
Gateway <--[CMP/UDP]-- Risk <--[CMP/UDP]-- ME
                       Risk --[SPSC]-----> PG write-behind
                                    ME --[SPSC]--> WAL Writer
                          WAL Writer --[notify]--> DxsReplay
                                    ME --[CMP/UDP]--> Marketdata
Mark --[DXS/TCP]------> Risk
Recorder --[DXS/TCP]--> ME
```

## Performance Targets

| Operation | Target |
|-----------|--------|
| WAL append | <200ns |
| WAL flush (fsync) | <1ms per 64KB |
| WAL sequential read | >500 MB/s |
| Replay 100K records | <1s |
| CMP encode/decode | <50ns |
| UDP round-trip (same machine) | <10us |
