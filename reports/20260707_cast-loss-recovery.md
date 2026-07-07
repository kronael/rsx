# casting loss & outage recovery — 2026-07-07

What casting does when the wire drops packets: how far the live NAK path
stretches before it gives up, and how the TCP-replication cold path recovers a
gap too big for the reorder ring. Two benches, both public-API-only (rsx-cast
untouched), run on a contended dev box (debug build).

Source: `rsx-cast/benches/loss_degradation.rs`,
`rsx-cast/benches/outage_recovery.rs` @ 50c78f9. Reproduce:
`cargo bench -p rsx-cast --bench loss_degradation` /
`--bench outage_recovery`.

## 1. Live NAK loss tolerance

A relay drops every forwarded datagram — first transmissions *and*
retransmits — with probability `loss`. The NAK channel (receiver→sender) does
not cross the relay (one-way-lossy forward link, the realistic case). Records
are 64 B so retransmits are served from the RAM send ring in ~µs, isolating the
NAK protocol from the O(N) cold-WAL retransmit cost that 144 B fills pay.
Default 8-retry budget; debounce set above worst-case round-trip so a merely
*late* retransmit can't burn a retry. Flow-controlled under the 2048 reorder
ring. A gap that outruns 8 consecutive retransmit losses (~`loss^8`) makes the
live path give up (→ TCP-replay territory).

N = 10 000 records, 5 trials/rate:

| loss rate | delivered | median throughput | vs 0-loss |
|---|---:|---:|---:|
| 0%  | 5/5 | 102 804 rec/s | 1.00× |
| 1%  | 5/5 | 121 269 rec/s | 0.85× |
| 5%  | 5/5 |  41 839 rec/s | 2.46× |
| 10% | 5/5 |  13 650 rec/s | 7.53× |
| 20% | 5/5 |   3 388 rec/s | 30.3× |
| 25% | 5/5 |   2 037 rec/s | 50.5× |
| 30% | 5/5 |   1 377 rec/s | 74.7× |
| 40% | 1/5 (4 gave up) | 706 rec/s | 146× |

**Reliable delivery holds through ~30% sustained loss**, degrading only in
throughput (~75× slower by 30% as more gaps need multi-round repair). The
ceiling is where a gap hits 8 consecutive retransmit losses; it is
stream-length dependent (more records = more tail-risk — at N = 20 000 the knee
is already at 30%, 3/5). Either way this is orders of magnitude past the
"assumes ≤ 0.01% loss" figure a naïve NAK design implies: the retry budget, not
0.01%, is what sets the wall.

## 2. Outage recovery over TCP replication

A relay goes fully dark, building a gap far larger than the 2048 reorder ring.
The receiver returns sticky `Reconnect`; the bench catches up over TCP
replication (TLS) from a `ReplicationService` on the same WAL, `reset_after_
replay`s, and resumes UDP — the real Pattern-A lifecycle. The gap is paced to
`outage_secs × 5000` records (recovery latency is a function of gap size, not
wall-clock).

| scenario | gap (records) | recovery |
|---|---:|---:|
| A: six 1 s outages (one stream, WAL growing) | ~7 500 each | 262 → 786 ms |
| B: one 10 s outage | 52 501 | ~1.0 s |

Scenario A's recovery **drifts up across cycles** (262 → 268 → 458 → 711 → 714
→ 786 ms for a constant ~7 500-record gap) as the active WAL grows: the
replication server's `oldest_and_highest_seq` re-scans the whole active WAL per
connection (tracked as REPLAY-RESCAN-PER-CONNECTION in BUGS.md). A 52 k-record
gap recovers in ~1 s.

## Caveats

- Debug build, contended box, loopback — absolute throughput is a floor, not
  production. The *shapes* (degradation curve, the ~30% knee, the per-cycle
  drift) are the findings, not the wall-clock constants.
- Loss tolerance is measured on ring-served 64 B records. 144 B fills bypass
  the send ring, so their retransmits pay the O(N) WAL scan (see
  `loss_recovery` bench, `reports/…single-gap`): tolerance is then also
  latency-bound, not purely loss-bound.
- One-way-lossy only. A bidirectionally-lossy link (NAKs also dropping) would
  lower the ceiling.
