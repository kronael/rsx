---
status: shipped
---

# CMP — C Message Protocol

Fixed-size C structs over the network. One wire format for
disk, network, and memory. No IDL, no codegen, no
serialization step.

Two transport modes:
- **CMP/UDP** — hot path (order flow, fills). Lowest
  latency. Aeron-inspired NACK + flow control.
- **WAL replication over TCP** — cold path (WAL replay,
  replication). Plain TCP byte stream, optional TLS.

Both carry identical WAL records. Only the transport differs.

## Table of Contents

- [1. Design](#1-design)
- [2. Wire Format](#2-wire-format)
- [3. Transport: CMP/UDP (hot path)](#3-transport-cmpudp-hot-path)
- [4. Transport: WAL Replication over TCP (cold path)](#4-transport-wal-replication-over-tcp-cold-path)
- [5. Protocol Patterns](#5-protocol-patterns)
- [6. Known Pitfalls](#6-known-pitfalls)
- [7. Comparison](#7-comparison)
- [8. Implementation](#8-implementation)
- [9. Performance Targets](#9-performance-targets)
- [Cross-References](#cross-references)

---

## 1. Design

```
WAL bytes = disk bytes = wire bytes = memory bytes
```

A CMP message is a WAL record: 16-byte header followed by a
fixed-size `#[repr(C, align(64))]` payload. The same bytes
are written to WAL files, sent over the network, and read
into memory with zero transformation.

### Why CMP

CMP is raw fixed-record bytes with a 16B WAL header. No
serialization step, no schema codegen, no extra framing.

---

## 2. Wire Format

Every CMP message is a WAL record (see DXS.md section 1):

```
struct WalHeader {       // 16 bytes
    record_type: u16,    // message type enum
    len: u16,            // payload length in bytes
    crc32: u32,          // CRC32 of payload
    _reserved: [u8; 8],  // reserved for future use
}
```

Payload immediately follows header. All fields little-endian.
All payloads are `#[repr(C, align(64))]` with explicit
padding fields.

### CmpRecord convention

All data records implement the CmpRecord trait, which requires
a `seq: u64` field as the first 8 bytes of the payload.
Sequence numbers are monotonic per stream and are assigned by
WalWriter::append or CmpSender::send. Control messages
(StatusMessage, Nak, Heartbeat) do not carry `seq`.

### Maximum message size

`len` is u16 and capped at 64KB (`MAX_PAYLOAD`). Messages
larger than 64KB are rejected at the sender. This prevents
DoS via length field and keeps allocation bounded.

---

## 3. Transport: CMP/UDP (hot path)

For the live order/fill path between Gateway, Risk, and ME.
Lowest possible latency. One WAL record per UDP datagram.

### Wire format

```
[UDP datagram]
  [16B WalHeader][payload]
```

One record per datagram. No fragmentation — all payloads
are <=64 bytes (one cache line), well under MTU.

### Why UDP

- No connection setup (just sendto/recvfrom)
- No head-of-line blocking
- No congestion control (dedicated network, we control
  both ends)
- No TLS overhead
- Kernel bypass ready (DPDK/AF_XDP swaps sendto for
  direct NIC write)

### CmpRecord trait

All **data** records implement the CmpRecord trait:

```rust
pub trait CmpRecord: Copy {
    fn seq(&self) -> u64;
    fn set_seq(&mut self, seq: u64);
}
```

- First 8 bytes of every data payload are `seq: u64`
  (monotonic per stream).
- WalWriter::append<T: CmpRecord> and CmpSender::send<T:
  CmpRecord> assign `seq` before encoding.
- extract_seq(payload: &[u8]) reads first 8 bytes as u64.
- Control messages (StatusMessage, Nak, Heartbeat) do NOT
  implement CmpRecord and have no seq field.

### Control Messages

Three control message types for reliability and flow
control, inspired by Aeron's protocol design.

**Record types:**
```
RECORD_STATUS_MESSAGE = 0x10
RECORD_NAK            = 0x11
RECORD_HEARTBEAT      = 0x12
```

**StatusMessage** (receiver -> sender, every 10ms):
```
#[repr(C, align(64))]
struct StatusMessage {
    consumption_seq: u64,   // last fully received seq
    receiver_window: u64,   // bytes willing to receive
    _pad1: [u8; 48],
}
```

**Nak** (receiver -> sender, on gap detection):
```
#[repr(C, align(64))]
struct Nak {
    from_seq: u64,          // first missing seq
    count: u64,             // number of missing records
    _pad1: [u8; 48],
}
```

**CmpHeartbeat** (sender -> receiver, every 10ms):
```
#[repr(C, align(64))]
struct CmpHeartbeat {
    highest_seq: u64,       // last sent seq
    _pad1: [u8; 56],
}
```

### Flow Control (Aeron model)

- Sender tracks `consumption_seq + receiver_window`
  from the latest StatusMessage
- Sender won't send beyond that limit (backpressure)
- Receiver sends StatusMessage every 10ms
- If sender has no room, `send` returns `false`. Caller may
  stall/retry or drop based on policy.

### Gap Detection (Aeron model)

- Receiver expects sequential seq numbers per stream
- Heartbeat tells receiver the sender's highest_seq
- Gap detected: receiver sends Nak immediately
- Sender reads Nak, fetches missing records from WAL,
  resends as normal data records
- Retransmits are just normal data records re-read from
  WAL and re-sent. No special record type.

### Sender

```rust
pub struct CmpSender {
    socket: UdpSocket,
    dest: SocketAddr,
    stream_id: u32,
    next_seq: u64,
    peer_consumption_seq: u64,
    peer_window: u64,
    last_heartbeat: Instant,
    wal_dir: PathBuf,   // WalReader opened on-demand per NAK
}

impl CmpSender {
    pub fn send<T: CmpRecord>(&mut self, record: &mut T) -> Result<()> {
        record.set_seq(self.next_seq);
        self.next_seq += 1;
        // encode header + payload, sendto
    }
}
```

### Receiver

```rust
pub struct CmpReceiver {
    socket: UdpSocket,
    sender_addr: SocketAddr,
    stream_id: u32,
    expected_seq: u64,
    highest_seen: u64,
    reorder_buf: BTreeMap<u64, Vec<u8>>,
    last_status: Instant,
    window: u64,
}
```

Reorder buffer bounded at 512 slots.

---

## 4. Transport: WAL Replication over TCP (cold path)

For WAL replay, replication, and any bulk streaming where
throughput matters more than latency. Plain TCP byte stream.
Optional TLS via rustls.

### Protocol

1. Client connects via TCP (optionally TLS)
2. Client sends ReplayRequest (WAL record)
3. Server streams WAL records: `write_all(header)`,
   `write_all(payload)`, repeat
4. Client reads: `read_exact(16)`, `read_exact(len)`,
   repeat
5. Server sends `RECORD_CAUGHT_UP` when replay complete,
   then transitions to live broadcast

No additional framing. The 16-byte WAL
header provides all necessary framing (version, type,
length, CRC).

### Connection patterns

**Streaming (WAL replay, live tail):**
- Client sends single request record (stream_id, from_seq)
- Server writes WAL records continuously
- Unidirectional from server to client after handshake
- Server sends RECORD_CAUGHT_UP when replay complete,
  then transitions to live broadcast

**Fan-out (fills to multiple consumers):**
- One TCP connection per consumer
- Producer writes same records to each connection
- No pub/sub abstraction — explicit per-consumer streams

### Reconnect

Exponential backoff: 1s / 2s / 4s / 8s, max 30s.
Resume from `tip + 1`.

### TLS

Optional via rustls (config flag).

**Same machine (development/single-node):**
- No TLS needed (localhost)

**Cross-machine (production):**
- Enable TLS via config
- Self-signed certificate distributed to all nodes
- Optional: mutual TLS with per-node certificates

### Config

```
RSX_REPL_ADDR=10.0.0.1:9300
RSX_REPL_TLS=true
RSX_REPL_CERT_PATH=./certs/repl.pem
RSX_REPL_KEY_PATH=./certs/repl.key
```

---

## 5. Protocol Patterns

### 5.1 Order Flow (Gateway -> Risk -> ME) — CMP/UDP

```
Gateway                   Risk                    ME
   |                        |                      |
   |--[NewOrder]--UDP------>|                      |
   |                        |--[NewOrder]--UDP---->|
   |                        |                      |
   |                        |<--[Fill]------UDP----|
   |<--[Fill]--------UDP----|                      |
   |                        |<--[OrderDone]-UDP----|
   |<--[OrderDone]---UDP----|                      |
```

Same WAL record types used everywhere. No translation
between components. Each record is one UDP datagram.

### 5.2 WAL Replay — TCP

```
Consumer                          Producer
   |                                 |
   |--[ReplayRequest]--TCP-------->|
   |   {stream_id, from_seq}        |
   |                                 |
   |<--[WalRecord]-------TCP-------|
   |<--[WalRecord]-------TCP-------|
   |<--[WalRecord]-------TCP-------|
   |<--[RECORD_CAUGHT_UP]-TCP-----|
   |                                 |
   |   (live tail: new records as    |
   |    they are appended to WAL)    |
   |                                 |
   |<--[WalRecord]-------TCP-------|
   |<--[WalRecord]-------TCP-------|
```

ReplayRequest is itself a WAL record:
```
#[repr(C, align(64))]
struct ReplayRequest {
    stream_id: u32,
    _pad0: u32,
    from_seq: u64,
    _pad1: [u8; 48],
}
```

### 5.3 Gap Fill — CMP/UDP

```
Receiver                          Sender
   |                                 |
   | (detects gap via Heartbeat)     |
   |--[Nak]--UDP----------------->|
   |   {stream_id, from:41, count:1} |
   |                                 |
   |   (sender reads seq 41 from WAL)|
   |                                 |
   |<--[WalRecord seq=41]---UDP-----|
```

Gap fill uses the same UDP path. Sender reads from WAL
(already on disk), resends as normal data record.

---

## 6. Known Pitfalls

These are the trade-offs of using raw C structs on the wire.
All are accepted for an internal single-team exchange.

### 6.1 Endianness

All fields are little-endian. Works on x86/x86_64 and
ARM little-endian (aarch64 default). Would break on
big-endian architectures. We are x86-only.

**Mitigation:** `#[repr(C)]` with explicit field order.
Compile-time assert on `cfg(target_endian = "little")`.

### 6.2 Alignment and Padding

Compilers insert padding between struct fields for
alignment. Different compilers or platforms may pad
differently.

**Mitigation:** `#[repr(C, align(64))]` with explicit
`_pad` fields. Compile-time `assert_eq!(size_of::<T>(), N)`
for every wire type. All padding bytes set to zero.

### 6.3 Limited Schema Evolution

Cannot remove or reorder fields in existing record types
without breaking all readers.

**Mitigation:** New record types are additive. Readers ignore
unknown record types. Breaking changes (field reorder, type
changes) require coordinated deployment (stop all producers,
upgrade all readers, restart).

**Version policy:** New record types added without version
bump. Unknown record types are logged and ignored. Breaking
changes = coordinated deploy window.

**Upgrade order:** consumers first (they ignore unknown
record types), then producers.

### 6.4 Torn Reads

Partial write + crash = truncated record on disk or wire.

**Mitigation:** CRC32 in header covers payload. Readers
validate CRC and discard invalid records. WAL truncates at
first bad CRC. TCP handles partial reads at transport
level (reliable delivery).

### 6.5 Transmute Unsoundness

Casting arbitrary bytes to a Rust struct can be undefined
behavior if the struct has validity invariants.

**Mitigation:** Use `ptr::read` on Copy types only. Never
`transmute`. Wire types have no invariants beyond field
types (all integer/bool, no enums on wire).

### 6.6 Invalid Enum Values

A u8 field with value 7 when the enum has variants 0-5.

**Mitigation:** Wire types use raw integers (u8, u16),
not Rust enums. Conversion to enum happens at the API
boundary with explicit validation.

### 6.7 No Floating Point

NaN != NaN, platform-dependent representations, loss of
precision.

**Mitigation:** All prices and quantities are i64
fixed-point. Zero floats anywhere in the system. Conversion
to/from human-readable format at the API boundary only.

### 6.8 DoS via Length Field

Malicious header claims len = 4GB, reader allocates.

**Mitigation:** `MAX_PAYLOAD = 64KB`. Reader rejects any
header with `len > MAX_PAYLOAD` before allocating.

### 6.9 No Framing Beyond Header

TCP is a byte stream, not message-oriented.

**Mitigation:** 16-byte header with length field provides
framing. Reader always reads exactly 16 bytes first, then
exactly `len` bytes. No ambiguity.

### 6.10 No Cross-Language Support

C struct layout is Rust-specific. Other languages need
manual struct definitions.

**Mitigation:** Not needed. All components are Rust,
compiled from same repo. External consumers (if any)
would use the WebSocket JSON API, not CMP.

### 6.11 No Human Readability

Binary on the wire. Can't `curl` or `tcpdump` easily.

**Mitigation:** WAL dump tool that decodes records to JSON
for debugging. Structured logging at each component
boundary. In practice, we debug with tracing, not wire
captures.

---

## 7. Comparison

```
             Other       CMP/UDP     WAL/TCP
Use case     general     hot path    cold path
Framing      Envelope    WAL header  WAL header
Serialize    protobuf    zero-copy   zero-copy
TLS          optional    none        optional
Reliability  TCP         Nak+WAL     TCP
Latency      ~5us        ~200ns      ~100us
Complexity   high        low         low
```

### When to use which

| Path | Transport | Why |
|------|-----------|-----|
| GW <-> Risk <-> ME (live) | CMP/UDP | Lowest latency, same datacenter |
| WAL replay / replication | TCP (+TLS) | Reliable streaming, cross-DC |
| External clients | WebSocket JSON | Human-readable, public API |

---

## 8. Implementation

Crate: `rsx-dxs` (same crate, transport is implementation
detail).

```rust
// CMP/UDP hot path
pub struct CmpSender {
    socket: UdpSocket,
    dest: SocketAddr,
    stream_id: u32,
    next_seq: u64,
    peer_consumption_seq: u64,
    peer_window: u64,
    last_heartbeat: Instant,
    wal_reader: WalReader,
}

pub struct CmpReceiver {
    socket: UdpSocket,
    sender_addr: SocketAddr,
    stream_id: u32,
    expected_seq: u64,
    highest_seen: u64,
    reorder_buf: BTreeMap<u64, Vec<u8>>,
    last_status: Instant,
    window: u64,
}

// TCP cold path (WAL replication)
pub struct DxsReplayService {
    listener: TcpListener,
    tls_config: Option<rustls::ServerConfig>,
}

pub struct DxsConsumer {
    stream: TcpStream,
    tls_config: Option<rustls::ClientConfig>,
}

// Shared: same WalHeader, same record types,
// same encode/decode functions
```

Config:
```
# Hot path (CMP/UDP)
RSX_CMP_UDP_ADDR=127.0.0.1:9100

# Cold path (WAL replication over TCP)
RSX_REPL_ADDR=127.0.0.1:9200
RSX_REPL_TLS=false
RSX_REPL_CERT_PATH=./certs/repl.pem
RSX_REPL_KEY_PATH=./certs/repl.key
```

---

## 9. Performance Targets

| Operation | Target |
|-----------|--------|
| CMP message encode | <50ns (memcpy) |
| CMP message decode | <50ns (ptr::read) |
| UDP round-trip (same machine) | <10us |
| UDP round-trip (same datacenter) | <50us |
| TCP round-trip (same machine) | <100us |
| TCP round-trip (cross-datacenter) | <1ms |
| UDP sustained throughput | >1M msg/s |
| TCP sustained throughput | >500K msg/s |

---

## Cross-References

- DXS.md: WAL record format, record types, payload layouts
- WAL.md: flush/backpressure rules
- NETWORK.md: system topology, connection patterns
- TILES.md: tile architecture, intra-process IPC
- blog/cmp.md: rationale and pitfalls
