# KCP / Quinn (QUIC) protocol compare — polish + oracle review

Sprint: 22-PROTOCOL-COMPARES (KCP, Quinn slice).
Scope: polish the existing `compare/kcp.md`, `compare/quinn.md` docs
and `benches/compare_kcp.rs`, `benches/compare_quinn.rs` benches; run
codex oracle on each bench; fix anything that biases the measurement.

## Files touched

- `rsx-dxs/compare/kcp.md` — rewritten (137 → ~300 LOC).
- `rsx-dxs/compare/quinn.md` — rewritten (89 → ~290 LOC).
- `rsx-dxs/benches/compare_kcp.rs` — rewritten (252 → ~340 LOC).
- `rsx-dxs/benches/compare_quinn.rs` — rewritten (211 → ~320 LOC).
- `rsx-dxs/compare/README.md` — updated KCP + Quinn rows with
  measured numbers; payload-size note updated to 128 B.

Cargo.toml: no changes needed (kcp 0.6, quinn 0.11, rcgen 0.13,
rustls 0.23 already present).

## Commits

- `57e18dd [dxs] kcp: polish doc + bench, oracle-reviewed, 128 B payload`
- `32ad545 [dxs] quinn: polish doc + bench, oracle-reviewed, 128 B payload`
- README rows landed inside `6053a07` (parallel-agent merge with my
  pending changes already on disk).

## Oracle findings — KCP

Files reviewed: `compare_kcp.rs`, `cmp_rtt_bench.rs`. Oracle returned
six findings.

### Taken

1. **Payload mismatch.** KCP bench used `[u8; 64]`; CMP RTT bench
   sends `FillRecord` which is 128 B (`size_of::<FillRecord>() ==
   128`). Switched KCP to a 128 B payload and renamed the bench IDs
   to `*_128b` for clarity.
2. **ACK timing gap (spin variant).** Client never called `flush()`
   after `kcp.recv()` ingested the echo, so the standalone ACK for
   that echo was deferred until the next outbound DATA frame. Added
   `kcp.flush()` + `drain_output()` immediately after the recv loop
   confirms a message, so the ACK emission is on the measured
   critical path.
3. **Adapter overhead bias.** The `UdpOutput` shim used
   `Arc<Mutex<VecDeque<Vec<u8>>>>`. The Kcp instance and the drain
   are both touched only by the owning thread — `Arc<Mutex>` was
   unnecessary contention. Switched to `Rc<RefCell<VecDeque>>`. The
   `Vec<u8>::from(buf)` alloc per frame remains (fundamental to the
   crate's callback API); documented as the residual adapter cost.
4. **Soft failure handling.** Replaced every `let _ = kcp.input(...)`,
   `kcp.send(...).unwrap()`, `kcp.flush().unwrap()` with `.expect("...")`
   carrying a descriptive message. Replaced the warmup/iter silent
   timeout `break`s with infinite spin (the server is busy-spinning,
   so no risk of deadlock) plus a 2 s warmup-only safety panic.
5. **Bootstrap fix.** Discovered while running: the Rust `kcp` crate
   requires at least one `update()` before `flush()` returns Ok
   (otherwise `Error::NeedUpdate`). Added one `update()` per side
   at startup so the hot loop's `flush()` calls bypass the scheduler.

### Skipped

6. **Naive variant as headline.** Oracle suggested the naive variant
   measures the polling model, not pure protocol cost, so it
   shouldn't be the comparison-to-CMP number. Agreed; the doc now
   explicitly labels `kcp_rtt_naive_1ms_interval_128b` as a
   "realistic integration mode" datapoint, NOT the apples-to-apples
   number. The spin variant is the comparison-to-CMP number.

## Oracle findings — Quinn

Files reviewed: `compare_quinn.rs`, `cmp_rtt_bench.rs`. Oracle
returned eight findings.

### Taken

1. **Error masking / partial reads.** Replaced `unwrap_or(0)`,
   `is_err()`-returns-`0`, and `let _ = write_all(...)` with
   `.expect()`. New-stream variant now uses `read_to_end(PAYLOAD_LEN)`
   bounded by FIN (client calls `finish()`); persistent variant uses
   `read_exact(buf)` of fixed size. Asserts on partial reads.
2. **No readiness barrier.** Added an explicit warmup RTT before
   each `b.iter` body. For the persistent variant this is also
   what causes the server's `accept_bi()` to fire (QUIC doesn't
   materialise the stream until first byte is sent).
3. **`rt.block_on()` per iter.** Documented as an unavoidable
   asymmetry vs CMP's synchronous spin-poll path. Cannot be
   removed without forking Quinn. Doc explains that published Quinn
   numbers (picoquic, iggy) include the same overhead, so the
   QUIC-vs-published comparison is fair; the QUIC-vs-CMP comparison
   is biased upward against CMP.
4. **Payload mismatch.** Switched to 128 B to match `FillRecord`.
   Renamed bench IDs to `*_128b`.
5. **Persistent variant differed from new_stream by two axes.** The
   old persistent variant added 4 B length-prefix framing; the
   new_stream variant did raw stream I/O. New persistent uses
   `read_exact(PAYLOAD_LEN)` — no extra framing, just the fixed-
   size record. Both variants now differ only by stream creation.

### Skipped

6. **TLS handshake outside timed loop.** Already correct.
7. **Connection reused, not re-established.** Already correct.
8. **Stream creation in new_stream variant + persistent variant
   exists.** Already correct.

## Bug fixes that affected measurement

Two real bugs surfaced during running:

- **KCP `flush()` returned `NeedUpdate`** because the Rust port
  requires `update()` once first. Initial spin-variant run
  panicked. Fixed with bootstrap `update()`. Re-ran: spin p50
  dropped from ~159 µs (when `flush()` was failing and the bench
  was actually measuring the panic-recovery loop) to ~17 µs once
  flush was working.
- **Quinn `accept_bi()` timed out** in the persistent variant
  because the order was `client open_bi → server accept_bi → spawn
  echo task → write/read`. QUIC's `accept_bi` only resolves once
  the client sends data, but the server task wasn't spawned yet to
  drive frames. Fixed by spawning the server echo task BEFORE
  `cli_conn.open_bi()`, then doing the warmup write which triggers
  accept_bi inside the task.

## Measured numbers (this host, 2026-05-24)

Loopback, 128 B payload, Criterion `--sample-size 10 --measurement-time 2`
for sanity-check runs.

| Bench | p50 |
|---|---|
| `cmp_rtt_fill_echo` (CMP, 128 B, from LANDSCAPE.md) | 10.3 µs |
| `tcp_rtt_nodelay_128b` | ~14 µs |
| `kcp_rtt_spin_flush_128b` | ~17 µs |
| `quinn_rtt_persistent_128b` | ~37 µs |
| `quinn_rtt_new_stream_128b` | ~38 µs |
| `kcp_rtt_naive_1ms_interval_128b` | ~11 ms |

Ordering matches the protocol-cost model:
CMP (no framing parse) < TCP nodelay < KCP turbo (24 B header +
ACK list) < Quinn (varint framing + AES-GCM) < KCP timer-driven
(scheduler granularity = 1 ms).

The 11 ms naive KCP number is dominated entirely by the
`sleep(1ms)` polling on each side — that is the integration mode
most apps actually use.

## Honest assessment

### Where KCP is genuinely better than CMP

- Portability: 1 000 LOC of standards C, ports in 8+ languages.
  CMP is Rust-only.
- Battle-tested under loss (gaming, KCPTun, FRP). CMP has not
  been deployed on a lossy WAN.
- No persistence dependency — drop-in transport. CMP assumes a WAL.

### Where Quinn (QUIC) is genuinely better than CMP

- Authenticated + encrypted transport (TLS 1.3) — when the threat
  model includes the network, this is a feature not overhead.
- Multiplexed streams without head-of-line blocking.
- Connection migration (client IP changes survive).
- NAT traversal works out of the box.
- Standardised wire format (RFC 9000/9001/9002).
- Ecosystem: every browser speaks QUIC; nothing speaks CMP.

### Where CMP is genuinely better than both

- Loopback / LAN latency: 10.3 µs vs KCP ~17 µs vs Quinn ~37 µs.
- Zero handshake (first packet is real payload).
- Audit log built in (WAL = wire = disk format; one stream feeds
  retransmit, audit, backtester, ML training).
- 48 h retransmit horizon via WAL random access (vs KCP/Quinn's
  bounded in-RAM horizon).
- Survives sender restart via WAL replay.
- Zero control-plane traffic on success (vs KCP's per-DATA ACK
  and QUIC's ACK + MAX_DATA frames).

## Open questions

- Should we measure `rt.block_on()` overhead in isolation and
  subtract it from the Quinn/TCP numbers? The doc admits the
  asymmetry but doesn't quantify it. A separate micro-bench of
  `rt.block_on(async {})` would give a baseline to subtract.
- The KCP spin variant's ACK flush is now on the critical path,
  but the ACK won't actually be processed by the server until the
  NEXT iteration's recv. So the ACK emission cost IS measured but
  the server's ACK-processing cost might be deferred. Subtle and
  arguably still measuring the right thing.
- No loss-injection numbers in this report. The bench supports
  `tc netem` injection but we didn't run it (needs root). A
  follow-up sprint should run with 0.1% / 1% / 10% loss to expose
  the regime where KCP's fast retransmit actually wins.
- `compare_all.rs` (uniform harness) still uses 64 B payloads
  inherited from before this sprint. Not touched per instructions.
  Should be aligned to 128 B in a follow-up.
