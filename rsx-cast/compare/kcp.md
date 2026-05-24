# KCP

Open-source reliable ARQ protocol over UDP by skywind3000 (Rui Kong).
~1 000 LOC of C in a single header pair (`ikcp.h` / `ikcp.c`).
MIT-licensed. Widely deployed in gaming, P2P (KCPTun), and VPN
tunnels. Explicitly *not* a TCP replacement — its README states
"sacrifice 10–20% bandwidth in exchange for a transmission speed
1.5×–2× of TCP" on lossy WAN paths.

Source: https://github.com/skywind3000/kcp (commit 1d8a8a4, 2025-02)

## Protocol

### Wire format — 24-byte header

```
Offset  Size  Field  Meaning
0       4     conv   Conversation ID (logical connection)
4       1     cmd    Command: PUSH=81, ACK=82, WASK=83, WINS=84
5       1     frg    Fragment index (0 = last/only; N = N more follow)
6       2     wnd    Sender's remaining receive window (in packets)
8       4     ts     Sender timestamp (ms, for RTT measurement)
12      4     sn     Sequence number
16      4     una    Cumulative ACK: all sn < una delivered
20      4     len    Payload length (0 for pure-ACK frames)
24+     var   data   Payload
```

All little-endian. MTU default 1400 B → MSS = 1376 B. Messages
larger than MSS are split into multiple segments with descending
`frg`; all segments must arrive before delivery.

Compare to CMP's `WalHeader` (`rsx-cast/src/header.rs`):
```
Offset  Size  Field
0       2     record_type
2       2     reserved (was payload_len)
4       4     crc32          (CRC32C of payload)
8       1     version        (V1 = current; V0 = legacy)
9       7     reserved
```
16 bytes total, fixed-size `#[repr(C, align(64))]` payload immediately
after, no `frg` field (CMP messages are pre-sized ≤ MTU; the
matching engine never produces frames > 256 B).

### Reliability model: ACK-based (sender-driven)

KCP detects loss at the **sender** via absence of ACKs:

1. Every received segment triggers an explicit `IKCP_CMD_ACK` back
   to the sender, plus a piggybacked `una` (cumulative ACK) on
   every outgoing frame.
2. When the sender sees ACK(N+2), ACK(N+3) but no ACK(N), it
   increments `fastack` on segment N.
3. `fastack >= resend` (turbo: `resend=2`) → **fast retransmit**
   without waiting for RTO.
4. If no ACK arrives within RTO → timeout retransmit.

Contrast with NAK-based (CMP, Aeron): the **receiver** detects the
gap on the next datagram with a higher seq and immediately sends
NAK(N). The sender retransmits in ~1 RTT.

Latency consequence on zero loss: KCP still sends one ACK per
DATA, so every DATA frame triggers a control-plane round-trip.
CMP on zero loss sends *zero* control traffic per record — only a
periodic `StatusMessage` every 10 ms for flow control
(`rsx-cast/src/protocol.rs` — RECORD_STATUS_MESSAGE).

### Retransmit horizon

| Property | KCP | CMP |
|---|---|---|
| Source of retransmit | `snd_buf` (RAM) | hot ring (4 096 slots, RAM) → cold WAL (disk) |
| Discard condition | Per-segment ACK received | Per-receiver consumption_seq via StatusMessage |
| Max horizon | Bounded by `snd_wnd` (default 32, turbo 128) | 4 096 hot, 48 h cold (WAL retention) |
| Survives sender restart | No | Yes (WAL replay) |
| Audit log | No | Yes (WAL = audit log) |

KCP discards a segment from `snd_buf` as soon as its ACK arrives.
A late NAK or a restarted receiver cannot recover any history.
CMP's cold-tier WAL provides 48 hours of random-access retransmit
via `read_record_at_seq` — long enough for a downstream service
to crash, restart, and resume from its last persisted offset.

### Flow / congestion control

KCP has two modes:
- **Standard** (`nc=0`): TCP-style CWND/ssthresh slow-start +
  congestion avoidance.
- **Turbo** (`nc=1`): no CWND. Sends as fast as `snd_wnd` allows.
  Receiver advertises its window in the `wnd` header field.

Turbo is correct for an exchange's trusted-LAN use case — a 10 GbE
datacenter fabric is not congested and TCP-style CC adds latency
without benefit.

CMP has no congestion control at all (spec §10.4: "Trusted internal
network"). Flow control is the receiver's `consumption_seq` carried
in `StatusMessage`; sender stalls when its window would exceed the
receiver's reported window.

### RTO

RFC 6298 SRTT/RTTVAR with KCP's modifications:
- Backoff: ×1.5 (vs TCP's ×2) — faster recovery from spurious timeouts.
- Min RTO: 30 ms in nodelay mode (`nodelay=1`), 100 ms otherwise.
- Integer millisecond precision throughout. **There is no
  sub-millisecond RTO.**

### Fastest configuration ("turbo mode")

```c
ikcp_nodelay(kcp, 1, 10, 2, 1);
//               ^  ^   ^  ^
//               |  |   |  nc=1: no CWND
//               |  |   resend=2: fast retransmit after 2 out-of-order ACKs
//               |  interval=10ms: scheduler tick floor
//               nodelay=1: immediate ACK + minRTO=30ms
```

The `interval` parameter governs `ikcp_update()`'s flush cadence.
The upstream KCP README recommends 10 ms; the Rust port allows
1 ms. Below 1 ms, `ikcp_check()` rounds to zero and `update()`
degenerates into a busy spin.

**Critical**: `interval` is the floor for the **scheduler**, not
for sends. Calling `ikcp_flush()` directly after `ikcp_send()`
bypasses the scheduler and writes to the socket immediately
(this is what the spin bench measures). The Rust `kcp` crate
also requires at least one `update()` call before the first
`flush()` (otherwise `flush()` returns `Error::NeedUpdate`); the
bench pays this once at startup.

### Connection model

KCP is **connection-less** at the wire level. A "connection" is
identified by the `conv` field and is just shared state on both
sides — no handshake, no SYN/FIN. The application is responsible
for telling KCP the peer's UDP address.

CMP is also connection-less (UDP unicast), identified by a
matching pair of bind addresses on sender and receiver. Spec
§10.4.

## Relation to rsx-cast

This is the answer to: *"why not just use KCP?"*

KCP is an excellent fit for its target problem: low-grade
networks (WAN gaming, mobile, P2P) where the underlying RTT is
20–300 ms and a 10× speedup over TCP under loss is competitive.
It has no business on an exchange critical path where:

1. The dominant latency is the `sendto` syscall (~3.85 µs
   measured locally, see `facts/syscall-latency.md`), not loss
   recovery.
2. Every per-DATA ACK doubles control-plane traffic vs CMP's
   NAK-on-gap model.
3. Integer-millisecond RTO is incompatible with a sub-100 µs SLA.
4. No persistence — a producer restart loses all retransmit
   history; CMP's WAL survives.

KCP also fundamentally lacks the audit-log property: every fill,
order, and cancel in rsx-cast is on disk before it's on the wire,
and the same bytes feed the recorder, the marketdata replay
service, and the backtester. KCP would be just a transport.

## Guarantees comparison: KCP turbo vs rsx-cast CMP

| Dimension | KCP turbo (`nc=1`) | rsx-cast CMP |
|---|---|---|
| Underlying transport | UDP unicast | UDP unicast |
| Wire header size | 24 B | 16 B |
| Loss detection | Sender (ACK absence + fastack) | Receiver (seq gap → NAK) |
| Detection latency (zero-loss) | n/a (ACK per DATA always) | n/a (no control plane on success) |
| Detection latency (1 lost frame) | ~2 RTT (need 2 newer ACKs) | ~1 RTT (gap seen on next frame) |
| Retransmit source | `snd_buf` (RAM, bounded by `snd_wnd`) | hot ring (4 096) + cold WAL (48 h) |
| Retransmit horizon | seconds (until ACK arrives) | 48 h |
| Survives sender restart | No | Yes (WAL replay) |
| Durability | None | WAL = audit log |
| Min flush granularity | 1 ms (Rust port) via timer; immediate via `flush()` | per `sendto` (~3.85 µs) |
| Multi-receiver / fan-out | No (one `conv` per peer) | Per-receiver via DXS TCP replay; CMP itself is unicast |
| Multiplexed streams | No (single seq space per `conv`) | No (one stream per CastSender/CastReceiver pair) |
| FIFO within stream | Yes | Yes |
| Cross-stream ordering | n/a | n/a (separate WAL files per producer) |
| Auth / encryption | None | None (trust delegated, spec §10.4) |
| Congestion control | Optional (`nc=0` standard / `nc=1` turbo) | None |
| Zero-loss control-plane overhead | One ACK per DATA | One `StatusMessage` per 10 ms |
| Heap allocation per send | Yes (`snd_buf.push_back`) | No (pre-allocated ring slot) |
| Language ecosystem | C reference + Rust port (`kcp` 0.6) + Go + many | Rust only (this crate) |
| Production HFT use | None documented | Target use case |
| Production gaming use | Extensive (KCPTun, FRP, ~10k stars) | None |

## Benchmark

`benches/compare_kcp.rs` (run with `cargo bench --bench
compare_kcp`) — Criterion, loopback, 128 B payload (matched
to casting's `FillRecord`, which is
`mem::size_of::<FillRecord>() == 128`).

Two scenarios:
- `kcp_rtt_naive_1ms_interval_128b` — timer-driven `update()` every
  1 ms on both sides. Shows the latency floor from the polling
  model. NOT a fair comparison with CMP's RTT bench (which
  spin-polls); kept as a "realistic integration mode" datapoint.
- `kcp_rtt_spin_flush_128b` — busy-spin server, explicit
  `flush()` after every `send()`. Reveals KCP's true protocol
  overhead with the scheduler bypassed.

Both use the same KCP configuration:
```
nodelay=1, interval=1ms, resend=2, nc=1, wndsize=128/128, mtu=1400
```

Loss simulation (separate run, requires root):
```bash
sudo tc qdisc add dev lo root netem loss 0.1%
cargo bench -p rsx-cast --bench compare_kcp
sudo tc qdisc del dev lo root
```
The bench itself does not depend on root or `tc`.

### What this bench is and isn't

This bench measures **application-visible loopback RTT** using
the same Criterion shape, payload size (128 B), and warmup
methodology as `cmp_rtt_bench.rs` — making `kcp_rtt_spin_flush_128b`
size-comparable to CMP's RTT bench (p50 ~10.3 µs on this host;
`.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`).

What it does NOT measure:
- Loss recovery (no `tc` injection in the bench itself).
- Multi-stream / fan-out (KCP is single-stream by design).
- WAN behaviour (loopback only).
- Memory / CPU under sustained load (single-iteration RTT only).

### Measured numbers (this host, 2026-05-24)

| Bench | p50 |
|---|---|
| `cmp_rtt_fill_echo` (CMP, 128 B) | 10.3 µs |
| `kcp_rtt_spin_flush_128b` | ~17 µs |
| `kcp_rtt_naive_1ms_interval_128b` | ~11 ms |

The 17 µs spin number is roughly 1.6× CMP — close to the lower
bound of KCP's possible per-frame overhead (24 B header parse,
ACK list maintenance, Rust port adapter copy). The 11 ms naive
number is dominated by the 1 ms sleep granularity on each side.

## Published numbers

From the KCP repository's own benchmark wiki (WAN, simulated
loss; sender + receiver on separate hosts):

| Protocol | Worst-case sample, 10% loss |
|---|---|
| KCP turbo | 195–295 ms |
| libenet | 1 412–1 637 ms |

KCP claims a 5–6× advantage over ENet under loss and 1.5×–2×
over TCP under "average" loss conditions. These are the headline
numbers KCP is known for; they are explicitly WAN/gaming.

Aeron loopback comparison
(`https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/`,
c6in.16xlarge, 100k msg/s):
- P50: 21–22 µs
- P99: 32–43 µs

CMP loopback RTT, this repo:
- P50: ~10.3 µs (`cmp_rtt_bench`, see LANDSCAPE.md).

KCP and Aeron / CMP do not compete in the same latency bracket
even on zero-loss loopback.

## Where KCP is genuinely better

- **Portability**: ~1 000 LOC of standards C; ports exist in Go,
  Rust, Python, Java, JS, Swift, C#. CMP is Rust-only.
- **Battle-tested on bad networks**: gaming and VPN deployments
  prove KCP works in production with 5–30% loss. CMP has
  never been tested on a public-internet path.
- **No persistence requirement**: KCP works fine with no disk;
  rsx-cast CMP assumes a WAL.
- **Multi-language reach**: if you need a client in C# or Swift,
  KCP wins by existing.

## Where CMP is genuinely better

- **Loopback / LAN latency**: ~10 µs RTT vs KCP's ~17 µs spin
  floor or millisecond timer-driven floor.
- **Audit log built in**: WAL is the same byte stream as the
  wire and disk format; one log feeds retransmit, audit,
  backtesting, and ML training.
- **Long retransmit horizon**: 48 h via WAL random-access vs
  bounded by ACK arrival.
- **Survives restarts**: WAL replay reconstructs sender state
  exactly; KCP's `snd_buf` is in-process RAM only.
- **Zero control-plane traffic on success**: no per-DATA ACK.

## Rust ecosystem

| Crate | Notes |
|---|---|
| `kcp` v0.6 | Pure Rust port; sync-friendly; MIT |
| `tokio_kcp` | Async stream API on top of `kcp` |
| `kcp-tokio` | Alternative async, claims zero-copy |

This bench uses `kcp` v0.6 (the most direct port of the C
reference) to keep the comparison close to the canonical
implementation.

## Sources

- KCP repo: https://github.com/skywind3000/kcp
- KCP English README:
  https://github.com/skywind3000/kcp/blob/master/README.en.md
- `kcp` crate: https://crates.io/crates/kcp
- KCP benchmark wiki: linked from the KCP repo README
- Aeron AWS 2025 numbers:
  https://aws.amazon.com/blogs/industries/aeron-on-aws-2025-performance-benchmark-results/
- CMP local loopback numbers:
  `.ship/18-COMPONENT-BENCHES/LANDSCAPE.md`, commit 82e9966 baseline
- Syscall floor: `facts/syscall-latency.md`
