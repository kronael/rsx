# SoupBinTCP

Nasdaq's reliable TCP framing for OUCH (order entry) and ITCH
(market data over TCP, e.g. for non-colo subscribers). Public
specification. Closest published peer to CMP's TCP framing — both
provide reliable, sequenced, length-prefixed records over TCP with
heartbeats and session resumption.

Spec: https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/soupbintcp.pdf

Why we include it: a real exchange wire protocol that fills the
same niche as `rsx-dxs` cold-path WAL replay. SoupBinTCP and DXS
TCP both add length-prefix framing + sequencing to a TCP stream;
benching them side-by-side answers "how much does the SoupBin
framing layer cost on loopback?"

## Protocol

### Wire format — 3-byte common header per packet

```
Offset  Size  Field        Meaning
0       2     length       Big-endian. Bytes that follow (excl. length).
                            Implies max packet payload = 65535 - 1 = 65534 B.
2       1     packet_type  ASCII letter — see table below.
3+      var   payload      Type-specific body.
```

All multi-byte integers are **big-endian** (network byte order).

### Packet types

| Letter | Direction | Meaning |
|---|---|---|
| `+` | C ↔ S | Debug packet (free-form ASCII, optional) |
| `A` | S → C | Login Accepted: session ID + sequence number |
| `J` | S → C | Login Rejected: reason code |
| `H` | C ↔ S | Heartbeat (server every 1 s; client every 1 s) |
| `Z` | S → C | End of Session (server done sending) |
| `L` | C → S | Login Request: user, password, requested session, requested seq |
| `O` | C → S | Logout Request |
| `R` | C → S | Client Heartbeat |
| `S` | S → C | Sequenced Data Packet (the actual ITCH/OUCH payload) |
| `U` | C → S | Unsequenced Data Packet (order entry, e.g. an OUCH message) |

The two payload-bearing types in steady state are `S` (server →
client, sequenced) and `U` (client → server, unsequenced).

### Sequencing model: TCP order + replay anchor

- **Sequenced** (`S`) packets are implicitly numbered: the seq
  number sent in the Login Accepted (`A`) packet plus the count
  of `S` packets received so far.
- **Unsequenced** (`U`) packets are best-effort; they do not get
  a sequence number. Order entry uses these — the client tracks
  its own client-side order IDs.
- After disconnect, the client reconnects with `L`, supplying
  `(session_id, next_seq)` to resume from a precise point. The
  server replays missing `S` packets in order, then resumes
  live streaming.

This is the same pattern as DXS TCP replay (`ReplayRequest{from_seq}`
→ live tail via `CaughtUpRecord`). SoupBinTCP's contract is a
strict superset only in that it adds the heartbeat and explicit
end-of-session markers.

### Heartbeats

Both sides send a Heartbeat packet (`H`/`R`, 3 bytes total) every
second when no other traffic is in flight. 15 seconds without any
packet from the peer → tear down the connection. This is the
exchange equivalent of CMP's `CmpHeartbeat` (every 10 ms by
default).

### Reliability: TCP's

SoupBinTCP delegates reliability to TCP. Loss detection, retransmit,
congestion control, ordering — all kernel-level. The framing layer
adds:
- Sequencing (replay anchor on reconnect).
- Heartbeat / liveness detection.
- Login / logout / session selection.

No application-level NAK. Loss within a session = TCP retransmit.
Loss across a disconnect = client reconnects with
`L{session, last_seq+1}` and the server replays.

## Relation to rsx-dxs

| Dimension | SoupBinTCP | rsx-dxs DXS (TCP cold path) |
|---|---|---|
| Transport | TCP (with `TCP_NODELAY` in practice) | TCP (with `TCP_NODELAY`) |
| Byte order | Big-endian | Little-endian (native) |
| Header size | 3 B per packet | 16 B per record (same as WAL on disk) |
| Sequencing | Implicit per S-packet count | Explicit `seq:u64` in every record |
| Replay anchor | `Login.requested_seq` | `ReplayRequest{from_seq:u64}` |
| Live tail signal | None (continuous stream) | `CaughtUpRecord{live_seq}` |
| Heartbeat | Every 1 s (`H`/`R`) | Every 10 ms (CMP only; DXS TCP uses TCP keepalive) |
| Login / auth | Username + password (cleartext) | None at transport (auth at gateway) |
| Encryption | None standard (TLS via OUCH-over-TLS extension) | None |
| Per-message framing overhead | 3 B | 16 B (record header = same as on-disk WAL) |
| Disk archive format | Application-defined (typically Nasdaq Glimpse for snapshots) | Identical to wire format (WAL = wire) |

### Stronger than DXS

- **Tighter framing.** 3-byte header vs DXS's 16-byte WAL header.
  At 10 M msg/s this is 130 MB/s of header-only bytes saved.
- **Auth is in the protocol.** Cleartext password isn't fit for
  external use, but it's a documented place to put credentials.
  DXS punts auth entirely to the gateway layer.
- **Explicit session + end-of-session.** Reduces ambiguity
  around "are we still streaming?" DXS uses `CaughtUpRecord` for
  the snapshot→live transition but has no equivalent "stream is
  over" marker.

### Weaker than DXS

- **Big-endian framing** on every parse. DXS reads native
  little-endian directly into `#[repr(C)]` structs — zero-copy
  payload access on x86_64.
- **No durable retransmit horizon.** SoupBinTCP relies on the
  exchange's archive service (Nasdaq Glimpse, a separate
  protocol). DXS guarantees 48 h of replay from the embedded WAL.
- **No application-level CRC.** SoupBin trusts TCP's 16-bit
  checksum. DXS records carry a CRC32 in the header — catches
  WAL corruption in addition to wire corruption.
- **Implicit sequencing** (S-packet count) is fragile: any
  out-of-band recovery has to count packets, not read sequence
  numbers directly. DXS records carry the seq in the payload.

## Benchmark

`../benches/compare_soupbintcp.rs` — Criterion, loopback, 64 B
payload over std `TcpStream` with `TCP_NODELAY`.

Frames each direction as `length:u16 (BE) | type:u8 ('U' or 'S')
| payload`. Sender writes the framed packet, echoer reads
3-byte header, then reads `length - 1` payload bytes, then
frames its own SoupBin packet back. Both directions perform a
full SoupBin parse + emit (not raw byte echo).

Expected p50 on Linux loopback: ~10–30 µs (raw TCP loopback
floor with `TCP_NODELAY` is ~10 µs std-sockets, framing parse
on both sides adds a few hundred ns).

## Sources

- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/soupbintcp.pdf (official spec)
- https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/ouch5.0.pdf (OUCH order entry, framed over SoupBinTCP)
- https://github.com/martinsumner/soupbintcp (Erlang reference, MIT)
- https://www.lseg.com/content/dam/data-analytics/en_us/documents/trading/turquoise/turquoise-soupbintcp-specification.pdf (LSE Turquoise reuses SoupBinTCP — second-source spec)
