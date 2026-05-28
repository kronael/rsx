---
sources:
  - local measurements (rsx-cast benches, 2026-05-28)
  - https://github.com/kronael/rsx/blob/master/facts/cmp-vs-udp-overhead.md
date: 2026-05-28
status: verified
local_measurement: true
---

# casting vs raw-UDP overhead

Host: AMD Ryzen 9 5950X (6-core slice), Linux 6.1, NVMe-backed ext4.
Rust release profile. Threads pinned to cores 2/3 via `core_affinity`.
All p50 from Criterion 100-sample runs.

## Hot path (UDP + protocol overhead)

| Bench | Measured | Command |
|---|---:|---|
| Raw UDP RTT, 128 B loopback | 9.89 µs | `cargo bench -p rsx-cast --bench cast_bench -- compare_udp` |
| casting RTT, 128 B loopback | 9.7 µs | `cargo bench -p rsx-cast --bench cast_rtt_bench` |
| casting overhead vs raw UDP | **~0 µs** | delta (within bench noise) |
| `CastSender::send` body | ~3.65 µs | `cargo bench -p rsx-cast --bench cast_send_breakdown_bench` |
| ↳ of which: `sendto` syscall | 3.61 µs | 99% of send body |
| ↳ of which: CRC32C, 128 B | 31 ns | |
| ↳ of which: header build | ~1 ns | |

The protocol adds negligible overhead on top of raw UDP.
The `sendto` syscall dominates at 99%.

## WAL (disk I/O)

| Operation | Measured | Command |
|---|---:|---|
| `WalWriter::append` in-memory | 31 ns | `cargo bench -p rsx-cast --bench wal_bench` |
| WAL flush + fsync, 1 record | 498 µs | `cargo bench -p rsx-cast --bench wal_fsync_bench` |
| WAL flush + fsync, 100 records | 627 µs | same — fsync dominates |
| WAL flush + fsync, 1 000 records | 1.19 ms | same |
| WAL flush + fsync, 10 000 records | 5.94 ms | same — append overhead visible |
| WAL sequential read | ~700 MB/s | `wal_bench` |
| Cold random read, 10 K records | 10.4 ms | `cargo bench -p rsx-cast --bench wal_random_read_bench` |
| Cold random read, 100 K records | 80.6 ms | same |

## Notes

- **p99 not measured.** All numbers are p50 from Criterion.
- **Loopback ≠ production.** The exchange's cross-process p50 is ~1 128 µs
  — dominated by monoio sleep(100 µs) polls, tokio reactor schedules, and
  PG write-behind. Transport overhead is ~10 µs of that.
- fsync latency is disk-dominated; batch size above ~100 records amortizes
  the cost linearly. Ten thousand records per flush (~5.94 ms) is the
  maximum practical batch for a 10 ms flush interval.
