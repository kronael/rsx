# Quinn (QUIC)

The dominant Rust QUIC implementation. QUIC (RFC 9000) is a
UDP-based transport with mandatory TLS 1.3, multiplexed
bidirectional streams, connection migration, and pluggable
congestion control. Quinn is used by Solana TPU, iroh, n0, and
many other Rust network projects.

Crate: https://crates.io/crates/quinn (v0.11, MIT/Apache-2.0)

## Protocol

QUIC runs over UDP but adds, on top of every datagram:

- **TLS 1.3** on every connection — encryption + integrity are
  mandatory and cannot be disabled (RFC 9001).
- **Connection handshake**: 1 RTT minimum to first byte of payload;
  0-RTT for resumed sessions with replay risk.
- **Multiplexed streams**: many independent bidirectional or
  unidirectional streams over one UDP 4-tuple. Each stream has
  its own seq space; head-of-line blocking is per-stream, not
  per-connection.
- **ACK-based loss recovery** (RFC 9002): QUIC's ACK frames carry
  packet-number ranges (richer than TCP SACK). Per-stream
  ordering; cross-stream HOL blocking eliminated.
- **Congestion control**: CUBIC (default in Quinn), with BBR and
  NewReno selectable via
  `quinn::TransportConfig::congestion_controller_factory`.
- **Connection migration**: client IP/port change survives
  transparently via Connection IDs (RFC 9000 §5.1).

### Wire format — variable-length, frame-in-packet

QUIC packets are not fixed-layout structs. Every packet is a
UDP datagram that contains:

```
QUIC packet (long header, used during handshake):
  1B   header_form + fixed_bit + long_packet_type + reserved
  4B   version
  var  dest_conn_id_len + dest_conn_id
  var  src_conn_id_len + src_conn_id
  var  token (Initial only)
  var  length (varint, payload length)
  var  packet_number (1-4 bytes)
  var  encrypted payload containing one-or-more frames

QUIC packet (short header, post-handshake 1-RTT):
  1B   header_form + fixed_bit + spin_bit + reserved + key_phase + pn_length
  var  dest_conn_id (length agreed during handshake)
  var  packet_number (1-4 bytes)
  var  encrypted payload (frames)
```

Frames inside the payload include STREAM, ACK, CRYPTO,
RESET_STREAM, MAX_DATA, PING, … (29 frame types in RFC 9000
§19). A single STREAM frame carries:

```
  1B   frame_type    (0x08..0x0f, low bits = OFF/LEN/FIN flags)
  var  stream_id     (varint)
  var  offset        (varint, if OFF flag)
  var  length        (varint, if LEN flag)
  var  stream_data
```

Compare to CMP, where one record = one UDP datagram = 16 B
`WalHeader` + fixed payload, no nesting, no varints, no
encryption.

The variable-length encoding is the price of QUIC's flexibility
(multiplexed streams, varint sizes, optional frame fields). CMP's
fixed layout means the receiver can compute every field's offset
at compile time and `read_record_at_seq` is a single `pread`
with no parser.

### Reliability model: ACK-based with packet-number ranges

QUIC tracks loss per-packet, not per-stream. Each packet has a
monotonic packet number (different from STREAM offset). ACK
frames acknowledge ranges of packet numbers. Loss is inferred
when:

1. A packet is reported missing in three subsequent ACKs
   (RFC 9002 §6.1 fast retransmit), or
2. The probe timeout (PTO) fires.

If a packet contained STREAM frame data, the data is
re-transmitted in a new packet (with a new packet number) —
unlike TCP where the sequence number is the byte offset, QUIC
fully decouples packet number from stream offset.

Contrast with CMP NAK-based recovery: receiver sees a gap in
sequence numbers (CMP's seq lives in the record's payload, not
the header) and sends a NAK frame. One RTT to retransmit. No
ACK on success. See `rsx-dxs/src/protocol.rs` `Nak`.

### Retransmit horizon

Quinn holds unacked packets in `quinn-proto`'s
`SentPacket` table (RAM). The horizon is bounded by the
congestion window and the connection idle timeout (default
30 s, configurable). There is no disk-backed retransmit; if the
sender process exits, all unacknowledged stream data is lost.

CMP's cold-tier WAL gives 48 h of random-access retransmit
(`read_record_at_seq`), survives sender restart, and doubles
as the audit log + recorder feed.

### Flow / congestion control

QUIC's congestion controllers ship in `quinn-proto`:
- CUBIC (default)
- NewReno
- BBR (experimental, behind a feature flag)

All three are designed for the public internet. On a 10 GbE
LAN with near-zero loss, they add scheduling latency without
improving throughput. CMP has no CC at all (spec §10.4).

### Connection model + handshake

QUIC has a full connection lifecycle:
1. Client → Initial packet with ClientHello (encrypted with
   Initial keys derived from connection ID).
2. Server → Initial + Handshake packets with ServerHello +
   certificate + Finished.
3. Client → Handshake + 1-RTT packets.

On loopback this typically completes in ~150–400 µs depending
on the rustls / aws-lc-rs path; the bench excludes this by
opening a persistent connection in setup. 0-RTT is possible
for resumed sessions but adds replay-attack semantics that are
out of scope for an exchange.

CMP has no handshake. The first record on the wire is real
data.

### Durability

None. Same as KCP. The application is responsible for any
persistence; Quinn is purely a transport.

## Relation to rsx-dxs

This is the answer to: *"why not QUIC for exchange IPC?"*

QUIC's overheads are the right tradeoffs for the public
internet (Solana TPU, HTTP/3, iroh peer-to-peer). On a trusted
10 GbE fabric behind a firewall:

- TLS is paying ~200–500 ns per record (AES-GCM) and
  ~150–400 µs once per connection (handshake) to solve a
  problem the network layer already solves (VPC + L3
  firewall).
- Congestion control is paying scheduling latency to solve a
  problem that doesn't exist on a fixed-capacity LAN.
- The handshake is paying 1 RTT per new connection — CMP
  pays zero.
- Variable-length framing forces a parser on the hot path;
  CMP records are direct casts of `#[repr(C, align(64))]`
  structs.

The iggy project measured Quinn at ~1.97 ms vs TCP at ~0.99 ms
on localhost for 40-byte messages (iggy/#606). Our local
`compare_quinn` measurements (128 B, persistent stream, this
host) are ~37 µs — significantly faster than iggy's number,
likely because we use a current-thread Tokio runtime, a
pre-opened bidirectional stream, and `read_exact` instead of
length-prefix framing. Still ~3.6× CMP's 10.3 µs RTT.

## Guarantees comparison: Quinn (QUIC) vs rsx-dxs CMP

| Dimension | Quinn (QUIC) | rsx-dxs CMP |
|---|---|---|
| Underlying transport | UDP unicast | UDP unicast |
| Wire framing | Variable (varints, nested frames) | Fixed 16 B header + fixed payload |
| Auth / encryption | TLS 1.3 mandatory | None (trust delegated, spec §10.4) |
| Handshake | 1-RTT (0-RTT optional with replay risk) | None |
| Loss detection | ACK-based (packet-number ranges) | NAK-based (receiver-driven on gap) |
| Detection latency (zero loss) | n/a (ACKs always sent) | n/a (no control plane on success) |
| Detection latency (1 lost pkt) | ~3 ACKs or PTO | ~1 RTT |
| Retransmit source | `SentPacket` table (RAM) | hot ring (4 096) + cold WAL (48 h) |
| Retransmit horizon | seconds (until ACK or idle timeout) | 48 h |
| Survives sender restart | No | Yes (WAL replay) |
| Durability | None | WAL = audit log |
| Multiplexed streams | Yes (many bi/uni per connection) | No (one stream per Cmp pair) |
| Cross-stream HOL blocking | No (per-stream) | n/a |
| FIFO within stream | Yes | Yes |
| Connection migration | Yes (Connection IDs) | No (UDP 4-tuple is identity) |
| Congestion control | CUBIC/NewReno/BBR | None |
| 0-RTT resumption | Yes (with replay caveat) | n/a |
| Zero-loss control-plane overhead | ACK frames, MAX_DATA, … | One `StatusMessage` per 10 ms |
| Heap allocation per send | Yes (Vec, varint encoding) | No (pre-allocated ring slot) |
| Language ecosystem | C (msquic, quiche), Go (quic-go), Rust, … | Rust only |
| Production HFT use | None documented | Target use case |

## Benchmark

`../benches/compare_quinn.rs` — Criterion, loopback, 128 B payload
(matched to CMP's `FillRecord`).

Three scenarios:
- `quinn_rtt_new_stream_128b` — open a fresh bidirectional
  stream every iteration. Shows the per-stream creation cost
  (HTTP/3-style "one stream per request").
- `quinn_rtt_persistent_128b` — open one stream in setup,
  reuse it across iterations with a fixed-size `read_exact`
  framing. Shows the steady-state QUIC overhead with the
  handshake AND stream creation outside the timed loop.
- `tcp_rtt_nodelay_128b` — TCP `TCP_NODELAY` baseline, also
  persistent connection with `read_exact`.

The TLS handshake is **always** outside the timed loop. The
self-signed cert is generated once with `rcgen`. The connection
is established and (for `persistent_128b`) the stream opened
before Criterion starts sampling. A full warmup RTT happens
before timing in each variant.

### What this bench is and isn't

This bench measures **application-visible loopback RTT** with
the same Criterion shape, payload size (128 B), and warmup
pattern as `cmp_rtt_bench.rs` — making the numbers
size-comparable to CMP's RTT bench (p50 ~10.3 µs on this host;
`.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`).

One important asymmetry remains:

> `rt.block_on()` is on the timed critical path of every QUIC
> and TCP iteration. This adds Tokio executor / waker scheduling
> overhead (~hundreds of ns) that CMP does NOT pay — CMP's RTT
> bench is synchronous and spin-polls. This is fundamental to
> Quinn's async API surface and cannot be eliminated without
> forking the crate. Published Quinn numbers (picoquic 20 µs
> min, iggy ~1.97 ms avg) include the same overhead, so the
> comparison is fair against published QUIC data, even though
> it is biased upward against CMP's syscall-only path.

What it does NOT measure:
- TLS handshake latency (excluded by design; ~150–400 µs once).
- Connection migration / packet reordering.
- WAN behaviour (loopback only).
- Multi-stream throughput.

Loss simulation (separate run, requires root):
```bash
sudo tc qdisc add dev lo root netem loss 0.1%
cargo bench -p rsx-dxs --bench compare_quinn
sudo tc qdisc del dev lo root
```
The bench itself does not depend on root or `tc`.

### Measured numbers (this host, 2026-05-24)

| Bench | p50 |
|---|---|
| `cmp_rtt_fill_echo` (CMP, 128 B) | 10.3 µs |
| `tcp_rtt_nodelay_128b` | ~14 µs |
| `quinn_rtt_persistent_128b` | ~37 µs |
| `quinn_rtt_new_stream_128b` | ~38 µs |

CMP < TCP < Quinn — as the protocol overhead model predicts.
`new_stream` is only marginally slower than `persistent`
because Quinn keeps stream creation cheap once the connection
exists (no extra RTT, just a STREAM frame on the existing
connection).

## Published loopback numbers

| Source | Metric | Value |
|---|---|---|
| picoquic (Christian Huitema, 2024, Linux loopback) | RTT min | ~20 µs |
| picoquic (2024, Linux loopback) | RTT p50 typical | 20–400 µs |
| picoquic (2024, Linux loopback) | RTT outliers (scheduler) | up to 1 400 µs |
| iggy/#606 (40 B, localhost) | QUIC avg RTT | ~1 970 µs |
| iggy/#606 (40 B, localhost) | TCP avg RTT | ~990 µs |

picoquic and Quinn share the QUIC wire format (RFC 9000) — these
numbers are representative of protocol overhead, not
implementation. Our `compare_quinn` measurement (~37 µs p50)
sits inside picoquic's published 20–400 µs range.

## Why Quinn over s2n-quic / quiche

All three are production-quality. Quinn chosen because:
- Pure Rust, no C dependency.
- MIT/Apache-2.0 dual license.
- Most public loopback benchmark data available.
- Used by Solana / iroh — high-volume real-world deployments.

## Where QUIC is genuinely better

- **Authenticated transport built in** — TLS 1.3 is a feature, not
  overhead, when the threat model includes untrusted network.
- **Multiplexed streams without HOL blocking** — multiple
  independent flows over one UDP 4-tuple.
- **Connection migration** — client IP changes are transparent;
  CMP has no equivalent.
- **NAT traversal** — QUIC works through NAT; CMP requires
  L3 reachability.
- **Standardised wire format** — RFC 9000/9001/9002. CMP is
  proprietary.
- **Ecosystem reach** — every browser speaks QUIC; nothing
  speaks CMP.

## Where CMP is genuinely better

- **Loopback / LAN latency**: ~10 µs RTT vs Quinn's ~37 µs
  (measured) or hundreds-of-µs in published benches.
- **Zero handshake** — first packet is real payload.
- **Audit log built in** — same byte stream on wire, disk,
  retransmit, audit, backtester.
- **48 h retransmit horizon** via WAL random access.
- **Survives sender restart** — WAL replay reconstructs state.
- **No CC, no encryption, no varints** — predictable cost model.

## Sources

- RFC 9000 (QUIC core): https://www.rfc-editor.org/rfc/rfc9000
- RFC 9001 (QUIC + TLS): https://www.rfc-editor.org/rfc/rfc9001
- RFC 9002 (QUIC loss recovery): https://www.rfc-editor.org/rfc/rfc9002
- Quinn crate: https://crates.io/crates/quinn
- picoquic loopback measurements:
  https://www.privateoctopus.com/2024/10/13/RandomLoopbackDelaysSlowBbr.html
- iggy issue #606 (TCP vs QUIC localhost):
  https://github.com/apache/iggy/issues/606
- CMP local loopback numbers:
  `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`
