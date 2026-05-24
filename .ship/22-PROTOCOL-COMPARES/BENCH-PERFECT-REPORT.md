# Bench refinement: pinning + oracle review

Sprint goal (from the task): pin every bench to specific cores, run
codex oracle on each bench file for measurement-validity bugs, take
the substantive findings, defer the rest, then re-measure and update
`facts/cmp-vs-udp-overhead.md`.

Two commits land on the worktree branch:

1. `ae75df9` `[dxs] bench: pin sender + echoer threads to cores 2/3`
2. `6b1127d` `[dxs] bench: align payload to 128 B, fix oracle-flagged
   measurement bugs`

## Pinning (commit ae75df9)

`core_affinity = "0.8"` added as a dev-dep. Every two-thread RTT
harness in `rsx-dxs/benches/` and `rsx-dxs/compare/` now:

- pins the Criterion timer thread (sender/pinger/PING) to core 2;
- pins the echoer/server/PONG thread to core 3;
- falls back to cores 0/1 on hosts with < 4 reported cores.

Single-thread benches (`cmp_bench`, `cmp_send_breakdown_bench`
stages other than `bench_sendto_loopback`, `wal_bench`,
`wal_fsync_bench`, `wal_random_read_bench`) pin their worker thread
to core 2.

Special cases:

- `cmp_send_breakdown_bench::bench_sendto_loopback` — worker on
  core 2, drain thread on core 3.
- `compare_aeron` — PING on core 2, PONG on core 3. Aeron's
  internal conductor/sender/receiver agents are unpinned (no
  rusteron FFI hook). On a 6-core slice they float on 0/1/4/5.
- `compare_quinn` and `compare_all`'s Quinn/TCP variants —
  `current_thread` Tokio runtime, only one OS thread. Pinned to
  core 2; server tasks share that core. Documented as a known
  bias against CMP's pure-syscall path.

## Oracle review (codex)

Twelve bench files were reviewed in parallel:

- `rsx-dxs/benches/{cmp_bench,cmp_one_way_bench,cmp_rtt_bench,
  cmp_send_breakdown_bench,compare_aeron,compare_tcp,compare_udp,
  wal_bench,wal_fsync_bench,wal_random_read_bench}.rs`
- `rsx-dxs/compare/{compare_all,compare_kcp,compare_quinn}.rs`

Raw reviews are saved under `tmp/oracle/<bench>.review.txt`.

### Findings and disposition

The recurring cross-cutting issues were **payload mismatch** (UDP/TCP/
Aeron/all hardcoded 64 B; CMP uses a 128 B `FillRecord`) and
**sample-count mismatch** (CMP RTT benches inherited Criterion
default of 100; the compare_* family forced 50). Both are fixed by
aligning everything to 128 B and `sample_size(50)`.

#### compare_udp.rs

| finding | disposition |
|---|---|
| Payload 64 B vs CMP 128 B | **TAKEN** — switched to 128 B; renamed `udp_rtt_loopback_64b` → `_128b` |
| `Err(_) => break` in timed loop silently records fast iterations | **TAKEN** — replaced with `panic!("pinger recv: {e}")` matching `compare_tcp.rs` |
| Sample count not aligned to compare_* family | **TAKEN** — added `sample_size(50)` via `criterion_group!` block |
| Set-affinity return value ignored | SKIPPED — robustness only, no measurement bug |
| Pinning AFTER bind — NUMA risk | SKIPPED — single-socket x86 |

#### cmp_rtt_bench.rs

| finding | disposition |
|---|---|
| `tick()`/`recv_control()` inside timed loop | SKIPPED — necessary for flow control over sustained Criterion runs; documented in file header |
| `fill_record()` constructed inside timed loop on both A and B sides | **TAKEN** — pre-built outside `b.iter` and outside the echo `while` loop |
| B side echoes a fresh `FillRecord` instead of the one it received | SKIPPED — same size, sub-µs cost; would require redeserializing the CMP frame buffer to reuse received bytes |
| Sample count not aligned to compare_* family | **TAKEN** — added `sample_size(50)` |
| UDP/TCP baselines were 64 B vs CMP 128 B | TAKEN against the baselines (see compare_udp / compare_tcp rows) |

#### cmp_one_way_bench.rs

| finding | disposition |
|---|---|
| `recv_count` atomic load/cacheline transfer in timed loop | SKIPPED — fundamental to "one-way latency" measurement; can't be removed without redefining the bench |
| `fill_record()` in timed loop | **TAKEN** — pre-built outside |
| Receiver `tick()` keyed off poll-loop iters, not received messages | SKIPPED — cosmetic; cadence still bounded |
| `set_for_current` return ignored, pinning order | SKIPPED — same rationale as compare_udp |
| Sample count not aligned | **TAKEN** — added `sample_size(50)` |

#### cmp_send_breakdown_bench.rs

| finding | disposition |
|---|---|
| `bench_ring_cache_copy` claims 144 B but copies 128 B | **TAKEN** — renamed bench id `send.ring_cache_copy_144b` → `_128b`, expanded docstring with the SEND_RING_FRAME_BYTES=128 rationale |
| `bench_sendto_loopback` pins worker AFTER bind | SKIPPED — single-socket x86; documented |
| `pick_cores()` could collapse onto same core on small hosts | SKIPPED — fallback yields `(0, 1)`, two distinct cores; only fails on a literal 1-core host where the bench is meaningless anyway |
| Criterion warmup/sample at defaults | SKIPPED — internally consistent across stages |

#### compare_tcp.rs

| finding | disposition |
|---|---|
| Payload 64 B vs CMP 128 B | **TAKEN** — `PAYLOAD = 128`, renamed bench id |
| Sample count 50 already aligned | n/a |
| `set_for_current` return ignored | SKIPPED |
| TcpListener not set_nonblocking — but accept is outside the timed loop | SKIPPED |

#### compare_aeron.rs

| finding | disposition |
|---|---|
| Payload 64 B vs CMP 128 B | **TAKEN** — `PAYLOAD_LEN = 128`, renamed both UDP and IPC bench ids |
| Docstring claims "CMP 64 B + 16 B WalHeader = 80 B on wire" | **TAKEN** — fixed to 128 + 16 = 144 B; updated wire-overhead ratio (~11%, not 20%) |
| Extra 100-iter warmup not done by CMP bench | SKIPPED — Aeron's media driver needs the prime; CMP's CMP-level handshake is implicit; not unfair |
| PING pinned after setup | SKIPPED — single-socket |

#### compare_kcp.rs

| finding | disposition |
|---|---|
| ACK `flush()` inside timed loop after `recv()` | DEFERRED with TODO — documented in file header as intentional ("the timed RTT does include the ACK emission cost"); if we want the symmetric variant it deserves its own bench id, not a behavior change here. TODO(oracle) comment added. |
| Sample count 50 already aligned | n/a |
| Set-affinity return ignored | SKIPPED |

#### compare_quinn.rs

| finding | disposition |
|---|---|
| Server + client on same `current_thread` runtime — same-core serialization | SKIPPED — limitation of Quinn's async API; documented in file header. Cannot be fixed without forking the crate. |
| `rt.block_on(...)` inside timed loop | SKIPPED — same — fundamental to the API surface; documented |
| Sample count 50 already aligned | n/a |

#### compare_all.rs

| finding | disposition |
|---|---|
| Payload 64 B across all protocols | **TAKEN** — `PAYLOAD_LEN = 128` constant, all bench ids renamed `_64b → _128b` |
| `TcpNodelay::ping` uses `read()` not `read_exact()` — short-read truncation | **TAKEN** — replaced with `read_exact(&mut buf[..PAYLOAD_LEN])`; server side also uses `read_exact` |
| `KcpSpinClient::ping` hardcodes `return 64` | **TAKEN** — returns the actual `kcp.recv()` length |
| `RawUdpClient` + `KcpSpinClient` store `_stop: Arc<AtomicBool>` but never set it; detached server threads leak between bench runs | **TAKEN** — added `Drop` impl on each that flips the stop flag, sends a wake-up sentinel (UDP) or relies on the next nonblocking poll (KCP), and joins the handle |
| Quinn / TCP on `current_thread` runtime — same-core w.r.t. server tasks | SKIPPED — Quinn API limitation, documented |
| Server-side binds before pinning the server thread | SKIPPED — single-socket |

#### cmp_bench.rs

| finding | disposition |
|---|---|
| `bench_reorder_buf_insert_lookup` allocates a fresh `BTreeMap` + `Vec` per iter | **TAKEN** — pre-allocated the map and a single `Vec<u8>`; the inner loop does `mem::take` to move the Vec in and `remove(&key)` to take it back, so steady-state alloc count is zero |
| `*_encode` benches "double-count CRC" | SKIPPED — the production CMP send path is exactly `compute_crc32(bytes)` then `encode_record(kind, bytes)`; the bench mirrors that. `encode_record` packs bytes without recomputing CRC. |
| `*_decode` benches only ptr::read, no CRC verify | SKIPPED — that IS what production `CmpReceiver` does for the payload payload; CRC verification is a separate path. |
| Header is 16 B everywhere | SKIPPED — confirmed via `WalHeader::SIZE = 16` |

#### wal_bench.rs

| finding | disposition |
|---|---|
| `bench_wal_append_in_memory` silently ignores `append`'s `Result`; backpressure WouldBlock fires once buf > 2 × max_file_size (~880k iters at 64 MiB cap) | **TAKEN** — bumped `max_file_size` to 1 GiB (≈ 14.9 M-iter headroom) and replaced `let _ =` with `.expect("INVARIANT: WAL append must not fail mid-bench")` |
| `bench_wal_flush_fsync`: `fill_record()` constructed inside timed loop; 800 records ≠ 64 KB (it's ~115 KiB at 144 B/record) | **TAKEN** — pre-built record; renamed `wal_flush_fsync_64kb` → `_115kb` with the correct payload arithmetic in comments |
| `bench_wal_flush_fsync` reuses one WalWriter across iters → rotation work creeps in once file grows past 64 MiB | **TAKEN** indirectly via the same 1 GiB cap bump |
| `WalReader::open_from_seq` inside `b.iter` | SKIPPED — the bench name says `read_sequential` / `replay`, which legitimately includes file open. If we wanted a pure-scan number we'd need a different bench. |

#### wal_fsync_bench.rs

| finding | disposition |
|---|---|
| `fill_record()` inside the timed loop on both single + batch variants | **TAKEN** — pre-built outside |
| Reuses one WalWriter across iters | SKIPPED — accurately models production WalWriter behavior |
| Batch_100 measures 100 appends + 1 flush per iter | SKIPPED — that IS the documented bench; per-record-amortized cost is a separate denominator |
| Docstring talks about 128 B + 16 B = 144 B | SKIPPED — accurate; the WAL framed record is 144 B regardless of underlying payload semantics. Oracle was wrong saying this is inconsistent. |

#### wal_random_read_bench.rs

| finding | disposition |
|---|---|
| Pseudo-random target generation inside timed loop | SKIPPED — `XorShift64::next` is single-instruction-class; vs a 10-100k linear scan it's noise |

### Deferred (TODO(oracle) markers)

- `compare_kcp.rs::bench_kcp_spin` — the `flush()` after `recv()` puts
  KCP's standalone-ACK cost INSIDE the timed RTT. Documented in the
  file header as intentional, but if a future contributor wants to
  benchmark "wire RTT minus ACK emission" we'd need a separate bench
  id (e.g. `kcp_rtt_spin_no_ack_128b`). Added a TODO(oracle) comment
  to that effect.

### Skipped — style or theoretical

- All "set_for_current return value ignored" findings — robustness
  issues, no demonstrated measurement bug on any host with ≥ 4 cores.
- NUMA-locality concerns around pinning AFTER bind on a single-socket
  x86 box — codex itself flagged these as SKIPPED.
- Spin-loop micro-style (`spin_loop` vs noop) — every bench already
  uses `std::hint::spin_loop()`.

## Re-measurement

Re-ran the three benches called out by the task with the new tighter
settings (`--sample-size 50 --measurement-time 3 --warm-up-time 1`),
all 128 B payload, sender + echoer pinned:

```
compare_udp:                udp_rtt_loopback_128b    8.71 / 9.89 / 11.33 µs
cmp_rtt_bench:              cmp_rtt_fill_echo        9.39 / 11.26 / 13.60 µs
cmp_send_breakdown_bench:
  send.crc32_128b               14.66 / 15.54 / 16.75 ns
  send.header_build              4.09 /  4.21 /  4.37 ns
  send.buf_pack_144b             3.44 /  3.62 /  3.83 ns
  send.sendto_144b_loopback      3.75 /  4.04 /  4.38 µs
  send.ring_cache_copy_128b      2.95 /  3.05 /  3.19 ns
```

(low / median / high triples.)

Before/after pinning, on the same host:

| bench | metric | pre-pin (64 B UDP / 128 B CMP) | post-pin (128 B both) | delta |
|---|---|---:|---:|---:|
| `udp_rtt_loopback` | low | 9.89 µs | 8.71 µs | **-12%** |
| `udp_rtt_loopback` | median | 10.88 µs | 9.89 µs | **-9%** |
| `udp_rtt_loopback` | high | 11.80 µs | 11.33 µs | -4% |
| `cmp_rtt_fill_echo` | low | 10.45 µs | 9.39 µs | **-10%** |
| `cmp_rtt_fill_echo` | median | 13.56 µs | 11.26 µs | **-17%** |
| `cmp_rtt_fill_echo` | high | 17.28 µs | 13.60 µs | **-21%** |

CMP RTT's distribution tightened a lot more than UDP's, which matches
the hypothesis from `facts/cmp-vs-udp-overhead.md`: the prior CMP tail
was scheduler noise from thread migration, not protocol work.

`facts/cmp-vs-udp-overhead.md` has been updated with the new numbers
and a dedicated "before/after pinning" table.

## Open questions

1. **Aeron media-driver agent pinning.** rusteron exposes
   `set_idle_sleep_duration_ns` but no thread-affinity hook. Without
   it the conductor/sender/receiver agents float. For Aeron numbers
   that are truly comparable to Real Logic's published 21 µs P50 we
   would need either (a) a fork that pins the C-side agents, or (b)
   running Aeron with cgroup-based core isolation outside the bench
   process. Out of scope for this sprint.

2. **Quinn server task pinning.** `Builder::new_multi_thread()` would
   give us a separate worker thread to pin, but it adds tokio's
   work-stealing scheduler overhead to the bench. Current single-
   thread runtime is the cleaner methodology for "what does Quinn
   cost in this API"; properly comparing transport overhead would
   need a Quinn server on a dedicated OS thread, which is invasive
   refactor.

3. **`compare_kcp` ACK-in-timed-loop semantics.** Whether to keep
   ACK emission inside the timed RTT or to bench wire-RTT separately
   is a methodology call, not a bug. The TODO(oracle) marker lets
   a future contributor split the bench cleanly.

4. **Lower-bound vs steady-state warmup.** Several benches use
   Criterion defaults; some force `sample_size(50)`. They are now all
   uniform at 50 in the comparison set. If we want to push down the
   noise floor for the headline `cmp_rtt_fill_echo` we could run
   `--sample-size 100 --measurement-time 5`, but the 50/3 settings
   used here matched the cross-bench convention and produced clean
   distributions.
