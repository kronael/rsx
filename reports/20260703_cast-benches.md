# rsx-cast benchmark run — 2026-07-03

Recorded live, sequentially (one bench at a time, appended as it completes,
so a mid-run failure never loses earlier numbers). Cluster STOPPED first so
risk/ME busy-spin tiles don't contend cores 2/3 (bench pins client→2, echo→3).
6-core box, debug cluster off. Provenance: [lib]=real library, [reimpl]=our
clean-room from spec (may be wrong/unoptimized), [our]=rsx-cast itself.

Payload 128 B (= size_of::<FillRecord>), sample_size 50 across all.

| Impl | Kind | p50 RTT | bench | status |
|---|---|---|---|---|
| cmp_rtt_fill_echo | [our] | **8.8021 µs** | cast_rtt_bench | ok |
| moldudp64_rtt_loopback_128b | [reimpl] | **8.8053 µs** | compare_moldudp64 | ok |
| soupbintcp_rtt_loopback_128b | [reimpl] | **11.164 µs** | compare_soupbintcp | ok |
| raw_udp_128b | [lib] | **8.7487 µs** | compare_all | ok |
| kcp_spin_flush_128b | [lib] | **10.414 µs** | compare_all | ok |
| quinn_persistent_128b | [lib] | — | compare_all | ABORTED: BENCH-QUINN-ACCEPT-BI panic |
| aeron_rtt_udp_loopback_128b | [lib] | **77.310 µs** | compare_aeron | ok |

## Results (p50, this run)
- **casting (rsx-cast)** — **8.80 µs** `[our]` — at the raw-UDP floor.
- **raw UDP** — 8.75 µs `[lib]` (std sockets; the floor).
- **MoldUDP64** — 8.81 µs `[reimpl]` — ties casting; OUR clean-room impl.
- **KCP** — 10.4 µs `[lib]` (turbo).
- **SoupBinTCP** — 11.2 µs `[reimpl]` — OUR clean-room framing over TCP.
- **Aeron (UDP loopback)** — 77.3 µs `[lib]` — real media driver, high variance (48–108 µs).
- **Quinn / QUIC** — ABORTED (BENCH-QUINN-ACCEPT-BI panic at compare_all.rs:356).
- **TCP_NODELAY** — not reached (compare_all aborted at Quinn before the TCP case).

## One-way delivery — the honest single-trip number

casting is fire-and-forget **one-way** delivery (ME→marketdata,
risk_out→gateway). The RTT above is a *comparison* metric — it needs only one
clock, so every protocol measures on equal footing, and it mirrors the order
round-trip — but the true per-delivery cost is one hop:

- **`cmp_one_way_fill` — 4.74 µs p50** (`cast_one_way_bench`): `CastSender::send`
  → `CastReceiver::try_recv`, one cast hop, in-order, no NAK.

RTT (8.80 µs) ≈ 2 × one-way + echoer turnaround — so ~4.7 µs, not 8.8, is what a
single casting delivery costs. Read the RTT for order-path shape; the one-way
for delivery latency.

## Send-path breakdown (`cast_send_breakdown_bench`)

Where the send half of a delivery goes — the **`sendto` syscall dominates**;
everything above the kernel is single-digit ns:

| stage | p50 |
|---|---|
| `send.header_build` (seq + CRC + WalHeader) | **711 ps** |
| `send.ring_cache_copy_128b` | **2.87 ns** |
| `send.buf_pack_144b` | **3.38 ns** |
| `send.crc32_128b` (CRC32C over payload) | **29.3 ns** |
| `send.sendto_144b_loopback` (syscall) | **3.54 µs** |

The whole userspace framing path is ~36 ns; the `sendto` syscall is ~100× that.
The one-way 4.74 µs is essentially one `sendto` + one `recvfrom` + framing.

## WAL — write, fsync, replay (`wal_bench`, `wal_fsync_bench`)

rsx-cast's WAL is the retransmit source AND the audit/replay log — half the
crate. Write path:

| op | p50 |
|---|---|
| `wal_write/append_1rec` | **36.2 ns** |
| `wal_write/write_1m_no_flush` | **34.7 ms** (~29 M rec/s, buffered) |
| `wal_write/flush_800rec` | **896 µs** |

Fsync amortizes with the 10 ms flush batch — per-record cost collapses as
records-per-flush grows:

| flush every | p50 (batch) | per-record |
|---|---|---|
| 1 rec | 363 µs | 363 µs |
| 10 rec | 409 µs | 41 µs |
| 100 rec | 475 µs | 4.8 µs |
| 1 k rec | 940 µs | 0.94 µs |
| 10 k rec | 4.82 ms | 0.48 µs |

Replay (cold-path recovery), linear in records:

| replay | p50 |
|---|---|
| 10 k | 13.5 ms |
| 100 k | 122 ms |
| 1 M | 1.23 s |

## Caveats (honesty)
- **FAIRNESS BUG: MoldUDP64 + SoupBinTCP are UNPINNED** (`TODO(pinning)` never
  done) while casting/raw-UDP/KCP/Aeron pin client→core2/echo→core3. Their
  numbers are therefore NOT strictly comparable — pending the uniform-harness
  refactor (.ship/31). Idle box limits the distortion but it's real.
- `[reimpl]` (MoldUDP64, SoupBinTCP) measure OUR reimplementations, which may be
  incorrect or unoptimized — reference baselines, NOT the vendors' products.
- Quinn aborts → no QUIC number this run; fix BENCH-QUINN-ACCEPT-BI first.
- compare_all aborting at Quinn also cost the TCP_NODELAY row (ordering).
- Single 6-core box, loopback, cluster stopped. Not wire-to-wire. Run yourself.
