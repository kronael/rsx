# 2026-07-08 — full bench rerun (in-process round-trip + Criterion)

## What was measured

Full rerun of `bench-match-rt` (in-process GW→ME→GW round-trip) plus
`cargo bench` (all Criterion). Triggered to reconcile three docs that
disagreed on the round-trip floor (README 7.5/16.9 µs, `docs/benches.md`
9.58 µs, `reports/20260703_cast-benches.md` cast-RTT-alone 8.8 µs).

Box: 6-core dev host, no core isolation, release build, `std::net::UdpSocket`
(bench does not use monoio — the two send legs are classical `sendto`).
Command: `./target/release/bench-match-rt --n 20000 --warmup 1000`.

## In-process round-trip (`bench-match-rt`), per-stage p50/p95/p99 (ns)

| stage | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|
| gw_send | 3196 | 4759 | 5480 | 56626 |
| me_dedup | 70 | 300 | 1623 | 447910 |
| me_wal_accept | 110 | 271 | 2104 | 45094 |
| me_match | 110 | 270 | 401 | 17823 |
| me_wal_events | 120 | 1714 | 2896 | 74160 |
| me_send | 3276 | 4919 | 11011 | 165280 |
| **TOTAL** | **7824** | **12102** | **22291** | **459462** |

Headline: **7.82 µs p50 / 22.3 µs p99**. The two ~3.2 µs send legs are the
`sendto` syscall (~82 % of the round-trip); the compute stages
(dedup + accept + match + events) sum to **~410 ns**. This supersedes both
the README 7.5/16.9 and the `docs/benches.md` 9.58 µs.

## Selected Criterion numbers (this run)

| bench | p50 |
|---|---:|
| `match_ioc_vs_1k_asks` | 77 ns |
| pure match / small ops | ~60–70 ns |
| `compression_new` | 14.5 µs |
| `slab_free` | 9.4 ns |
| `recenter_10k_orders` | **346 µs** |
| `event_buffer_drain_100` | 4.34 µs |

`recenter_10k_orders` at 346 µs confirms the eager-recenter tail spike
(`BUGS.md RECENTER-EAGER-TAIL-SPIKE`): a full migration is O(occupied
levels/slots), hundreds of µs, versus a ~60 ns steady-state match.

## Conclusion

- Round-trip floor is **7.82 µs p50 / 22.3 µs p99** in-process; ~99 % of
  production (cross-process ~1.1 ms) is inter-process overhead, not algorithm.
- The compute path is ~410 ns; the rest is `sendto`. The io_uring/SQPOLL
  roadmap work targets the syscall legs, not the match.
- Recenter is a real, bounded (346 µs @ 10k orders) tail spike — logged.

## Caveats

- Single box, in-process (no real NIC, no process boundary), `std` UDP not
  monoio (overstates the send legs by ~2 µs each vs io_uring). p99/max are
  noisy on a shared dev box (the 447 µs `me_dedup` max is a scheduler blip).
- Numbers are a floor, not a production SLA. Cross-process p50 (~1.1 ms) is
  the honest end-to-end figure.

Source: `bench-match-rt` (rsx-cli), `cargo bench` workspace; commit at rerun
time on detached HEAD. Reconciled into README §How-fast + `docs/benches.md`.
