# rsx-cast Benches

Every measurement program in this crate: what it measures and how
to run it. Numbers in [README.md](README.md) and
[ARCHITECTURE.md](ARCHITECTURE.md) trace back to one of these
files; [`facts/cast-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cast-vs-udp-overhead.md)
records the dated measured values.

## How to run

```
# One Criterion bench
cargo bench --bench <bench_name>

# Quick smoke (50 samples, 3s measurement):
cargo bench --bench <bench_name> -- --sample-size 50 \
  --warm-up-time 1 --measurement-time 3

# All benches in this crate
cargo bench -p rsx-cast
```

Criterion writes per-bench results to
`target/criterion/<bench>/` (HTML + JSON).

## Bench inventory

| Bench | Measures | What it isolates |
|---|---|---|
| `compare_all::raw_udp_128b` | Raw UDP loopback RTT, 128 B payload, two non-blocking sockets spinning. **Absolute floor.** | Baseline: no protocol work |
| `cast_one_way_bench` | `send_framed` → `CastReceiver::try_recv` one direction | Hot send → hot recv |
| `cast_rtt_bench` | casting echo RTT (A → B → A), two paired senders + receivers | Full sender → echo → sender triangle |
| `cast_send_breakdown_bench` | Each step of the frame+send path (`Framed::pack` + `send_framed`): CRC, header build, buf pack, `sendto`, NAK ring copy | Attributes the ~4 µs send body — 99 % is `sendto` |
| `wal_bench` | `WalWriter::append` in-memory, flush + fsync 64 KB, sequential read 10 K records | Append (31 ns) + sequential reader throughput |
| `wal_fsync_bench` | `WalWriter::append` + explicit flush + fsync to disk | Durability cost: 498 µs p50 single-record |
| `wal_random_read_bench` | `read_record_at_seq(random)` over a pre-populated WAL | Cold-tier NAK retransmit path; in-file scan, 10.4 ms @ 10 K records |
| `cast_bench` | Protocol record encode/decode (NAK, Heartbeat) | Wire-level primitives only — not on the per-packet send path |
| `compare_all` | Same RTT harness against raw UDP + KCP + Quinn (QUIC) + raw TCP, one process | Apples-to-apples comparisons; bench IDs `raw_udp_128b` / `kcp_spin_flush_128b` / `quinn_persistent_128b` / `tcp_nodelay_128b`; see `compare/README.md` |
| `compare_aeron` / `compare_moldudp64` / `compare_soupbintcp` | Same RTT harness against Aeron, MoldUDP64, SoupBinTCP | Vendor-protocol comparisons |

All Criterion benches in this crate pin sender + echoer threads to
cores 2 and 3 (`core_affinity = "0.8"` dev-dep). Single-thread
benches pin their worker to core 2. See
[`facts/cast-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cast-vs-udp-overhead.md)
§ "The pinning gap" for the before/after distributions.

## Send-path sub-attribution (`cast_send_breakdown_bench`)

Per-stage median, 128 B payload + 16 B header, post-pinning:

| Sub-step | p50 |
|---|---:|
| `crc32_128b` | 29.4 ns |
| `header_build` | 0.7 ns |
| `buf_pack_144b` (two memcpys → buf) | 3.3 ns |
| **`sendto_144b_loopback`** | **3.59 µs** ← 99 % |
| `ring_cache_copy_128b` | 2.9 ns |
| **Sum** | **~3.63 µs** |

If every line of Rust in the send path were eliminated, you'd
save ~36 ns out of ~3 630 ns — ~1 % improvement. The remaining
~99 % is the `sendto` syscall, which is kernel code we don't
own. To reduce it: io_uring SQE submission, `sendmmsg` batching,
or kernel bypass (DPDK / AF_XDP). See
[`facts/syscall-latency.md`](facts/syscall-latency.md) for the
mechanism.

## Caveats and gotchas

- **`set_read_timeout` setsockopt inside a hot loop** adds ~µs
  per iteration. Why an earlier `udp_rtt` bench read as 29 µs
  for weeks before the fix.
- **Both sockets binding the same port + `SO_REUSEPORT`**
  hash-distributes incoming traffic. Each `CastSender` /
  `CastReceiver` needs its own port.
- **WAL fsync 498 µs (single record) is amortised in
  production**: the writer flushes every 10 ms, not per record.
  As long as ≥ 10 orders share one fsync, per-order cost is
  ≪ 498 µs. The ~24 µs `wal_flush_fsync_64kb` figure is the same
  flush amortised over a 64 KB batch.
- **Loopback p50 is not what production sees.** Cross-process
  production p50 is ~1 128 µs because of monoio's 100 µs
  sleep poll plus tokio reactor schedule plus PG write-behind
  churn. The bench numbers measure the protocol floor; the
  production-floor delta lives outside this crate.

## See also

- [`compare/README.md`](compare/README.md) — protocol survey and
  comparison harness (raw UDP, TCP, KCP, Quinn, Aeron, MoldUDP64,
  Chronicle Queue, LBM, SoupBinTCP).
- [`facts/syscall-latency.md`](facts/syscall-latency.md) — the
  syscall-level "why" behind the ~4 µs `sendto` cost.
- [`facts/cast-vs-udp-overhead.md`](https://github.com/kronael/rsx/blob/master/facts/cast-vs-udp-overhead.md)
  — authoritative dated measurements (parent repo).
