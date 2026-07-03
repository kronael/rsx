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
