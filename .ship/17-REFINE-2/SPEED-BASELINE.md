# Speed Baseline — 2026-05-22

Snapshot of measured latencies BEFORE any deep optimisation
work. Captured against the live cluster running commits
through `a2e9757` (post-FillRecord wire change + probe race
fix + cluster restart with new binaries).

Used as the reference point so we can attribute future
improvements specifically.

## Setup

- Host: dev box, Linux 6.1.0-43-amd64
- Build: debug profile (`cargo build`)
- Scenario: `minimal` (gateway + risk-0 + me-pengu +
  marketdata + mark + recorder + maker)
- Maker: `rsx-playground/market_maker.py`, 5 levels,
  20 bps spread, refresh 500 ms
- Probe: `rsx-playground/server.py::_run_latency_probe`,
  cross-price = 1.01 × best_ask, lot_size = 100_000
- N = 500 orders + 50 warmup
- Concurrent traffic: maker quoting at ~10 orders/s

## End-to-end probe (Python aiohttp WS round-trip)

```
e2e_us:
  count = 604
  p50   = 11 875 µs
  p95   = 24 551 µs
  p99   = 342 893 µs
  min   = 10 428 µs
  max   = 528 188 µs
```

## Gateway-only RTT (Python WS, no risk/ME)

```
gw_only:
  count = 51
  p50   =   301 µs
  p95   =   529 µs
  p99   =   888 µs
```

This is **Python aiohttp WS round-trip with a rejected
order** — gateway parses, fast-rejects, writes the error
frame back. No risk, no ME, no PG. So this is the lower
bound on what Python imposes for a single round-trip.

## Per-stage (Rust path, 553 coherent traces)

| Stage | p50 µs | p95 µs | p99 µs |
|-------|-------:|-------:|-------:|
| gateway_in | 0 | 0 | 0 |
| risk_in | 60 | 1 264 | 4 577 |
| me_in | 265 | 3 407 | 7 726 |
| me_out | 423 | 3 437 | 7 942 |
| risk_out | 462 | 4 054 | 10 874 |
| gateway_out | 1 128 | 6 513 | 14 264 |

## Per-leg deltas (p50)

| Leg | Δ µs |
|-----|-----:|
| gateway → risk (gateway_in → risk_in) | 60 |
| risk → ME (risk_in → me_in) | 205 |
| ME match (me_in → me_out) | **158** |
| ME → risk return (me_out → risk_out) | 39 |
| risk → gateway → WS (risk_out → gateway_out) | **666** |
| GW→ME→GW total | **1 128** |

## Microbench baselines (Criterion, isolated)

For comparison — these are the per-operation numbers
measured in isolation, in-memory, no IO:

```
match single fill:                54 ns
WalWriter::append (Vec extend):   31 ns
FillRecord encode:                23 ns
header_encode / _decode:          3 / 5 ns
nak_decode:                       9 ns
```

The single-fill match at 54 ns vs the 158 µs `me_in →
me_out` p50 is a **2 900× gap**. The bench measures pure
algorithm cost; production includes tracing emission,
2-3 WAL appends, format!() for oid, dedup hashmap, order
index update, and one tokio-runtime tick.

## Why the legs look the way they do (proximate explanation)

### `me_in → me_out` = 158 µs (vs 54 ns bench)

Between the two trace emissions in `rsx-matching/src/main.rs`
(lines 466 + 569) we run:

1. `dedup.check_and_insert` — FxHashMap operation, low ns
2. `OrderAcceptedRecord` WAL append — ~30 ns in-memory
3. `incoming = order_msg.to_incoming()` — struct copy
4. `process_new_order(&mut book, &mut incoming)` — the
   match (54 ns Criterion baseline) + book mutation
5. `write_events_to_wal` — loops over emitted events,
   one WAL append per event (~30 ns each); for a single
   fill that's ~1-3 events
6. `update_order_index(book.events(), ...)` — hashmap
   insert per event
7. Two `tracing::info!` emissions (me_in + me_out) carrying
   format!("{:016x}{:016x}", ...) for the oid

Dominant suspects:
- **`tracing::info!` macro itself is the largest single
  line item.** Default `tracing-subscriber` fmt layer
  formats the event, writes to a `Mutex<Stdout>` (or file)
  with locking + line-buffered flush. Per-event cost is
  typically 5-20 µs in debug builds. Two emissions = 10-40 µs.
- **`format!("{:016x}{:016x}", ...)` for the oid string —**
  allocates a 32-byte String per emission. ~100-300 ns each,
  plus the alloc churn. Two of them per stage emission.
- **WAL appends** are 30 ns each in the benchmark — but the
  benchmark doesn't measure the time spent inside the
  writer's internal BufWriter when the underlying file
  isn't already mmap-resident in the page cache.
- **`process_new_order` is NOT just match_single — it's
  the full insert-or-match decision tree.** For an order
  that crosses (probe orders all cross), it runs match;
  but for one that rests, it runs `insert_resting`. The
  Criterion 54 ns is one or the other, not both.

To attribute precisely we need sub-stage tracing (next
section).

### `risk_out → gateway_out` = 666 µs (60% of the Rust path)

Between `risk_out` (in `rsx-risk/src/main.rs`) and
`gateway_out` (in `rsx-gateway/src/route.rs`) we cross a
process boundary via CMP/UDP plus a gateway-side handoff
through the connection's `outbound: VecDeque<String>` queue:

1. **risk → gateway CMP/UDP send** — risk's `cmp_sender.
   send(&mut fill_record)` writes 128 bytes via sendto.
   Loopback UDP one-way ≈ 10-50 µs.
2. **gateway recv loop** (`rsx-gateway/src/handler.rs`)
   parses the FillRecord and calls `route_fill`.
3. **`route_fill`** (already inspected):
   - `oid_hex(...)` × 2 — format two 32-char hex strings.
   - `serialize(&WsFrame::Fill { ... })` — JSON encode of
     the fill. Single string allocation; serde_json.
   - `tracing::info!` emission for `gateway_out` —
     same overhead as discussed above.
   - `push_to_user(taker_user_id, msg.clone())` —
     **string clone** + push_back on the connection's
     `outbound` VecDeque.
   - `push_to_user(maker_user_id, msg)` — another clone
     for the maker (user 99).
4. **Per-connection handler loop wakes** — and *this is
   the dominant cost*. Looking at the loop body at
   `rsx-gateway/src/handler.rs:90-174`:
   ```
   loop {
       drain_outbound(conn_id);      // write pending msgs
       <heartbeat check>
       let ready = monoio::time::timeout(
           Duration::from_millis(10),
           stream.readable(false),
       ).await;
       if ready.is_err() { continue; }   // 10 ms tick
       ...read...
   }
   ```
   Between two iterations of the loop, the handler is
   blocked in the `readable()` await. When a fill is
   pushed to `outbound` AFTER the iteration's `drain_outbound`
   call (which is most of the time, since fills arrive
   asynchronously over UDP), the fill waits until the NEXT
   iteration — which is bounded by the 10 ms timeout or by
   the socket becoming readable.
5. **`ws_write_text`** then frames + writes the bytes to
   the TCP socket. Loopback write is fast (~10 µs).

So 666 µs decomposes (estimated, *not yet measured*):
- ~50 µs CMP/UDP one-way
- ~50 µs gateway parse + route_fill body
- ~10 µs serialize JSON
- ~30 µs tracing emission
- **~500 µs waiting for the per-conn handler loop to come
  around**
- ~30 µs ws_write_text actual write

The 10 ms readable-wait is a deliberate design choice
(comment at line 164: "This avoids io_uring cancel-safety
issues that corrupt the stream byte offset"). It bounds
the worst case but adds a uniform-random wait per
unconsumed fill. The fix isn't trivial because monoio's
cancel-safety story for `readable()` is what motivated
the timeout in the first place.

### `risk → ME` = 205 µs

UDP loopback send + recv: each direction ~30-50 µs.
Total round-trip via two CMP frames (risk → ME and back)
sums to 244 µs, in the right ballpark.

What's it doing? `risk-0` sends the order to `me-pengu`
via `cmp_sender.send(&mut OrderRequest)` — that's a sendto
syscall. ME's `cmp_receiver.try_recv()` is a recvfrom
syscall. Both can stall behind the tokio runtime's
scheduling tick.

### `gateway → risk` = 60 µs

Same shape as risk → ME but apparently faster. Probably
because gateway runs on monoio with io_uring (warm CPU
caches, batched submission) while risk runs on tokio with
classic epoll (one syscall per recv). The asymmetry
suggests a **port** from epoll to io_uring on the risk
side might cut its leg in half.

## Open hypotheses to validate with granular tracing

These are NOT measured yet — they're informed guesses
from reading the code. The next step (granular telemetry)
will confirm or refute each:

| Hypothesis | Predicted Δ µs |
|---|---:|
| The two `tracing::info!` emissions are the largest single line item inside `me_in → me_out` | 30-60 |
| `format!()` for hex oid strings allocates ~300 ns × 2 per stage | 0.5-1 |
| WAL writer's BufWriter has a hidden per-append cost beyond the benchmark | 10-30 |
| The 10 ms readable-wait dominates `risk_out → gateway_out` | 500-800 |
| `push_to_user`'s `msg.clone()` × 2 is meaningful | 5-20 |
| `serde_json` JSON encode of WsFrame::Fill is meaningful | 5-15 |

If hypothesis 4 is right, **the single biggest improvement
to GW→ME→GW latency comes from changing how the per-conn
handler is woken on outbound traffic** — replace the
`readable()` timeout with a futures-based "either socket
or outbound queue" wait. That would directly remove the
~500 µs leg dominator.

## How to reproduce this baseline exactly

```bash
cd /home/onvos/sandbox/rsx
# Cold start
curl -X POST 'localhost:49171/api/processes/all/stop?confirm=yes'
curl -X POST 'localhost:49171/api/processes/all/start?scenario=minimal&confirm=yes'
# Wait for maker to quote (~10 s after start)
N=500 make latency-publish
# E2E numbers land in bench-baseline.json
# Per-stage from log/{gw-0,risk-0,me-pengu}.log:
python3 <<'PY'
import re, statistics
from pathlib import Path
# (script in STAGE-LATENCIES.md "Reproduction" — joins by
# oid, filters coherent traces, prints p50/p95/p99)
PY
```

## What's NOT in this baseline (out of scope)

- Multi-symbol throughput (we test PENGU only)
- Sustained throughput under load (the maker is light;
  ~10 orders/s, not 10k)
- Cold-cache vs warm-cache (this baseline is warm — the
  ME's snapshot was loaded prior)
- Per-stage sub-microsecond gaps inside each component
  (the next sprint adds those)
- The probe's own Python overhead profiled (we know it's
  ~10.7 ms but haven't decomposed it; that's not a
  product number anyway)

## Checkpoint for future comparison

When we re-measure post-improvement, the exact metric to
beat:

```
gateway_out p50 = 1128 us   (target: ?)
me_in → me_out = 158 us     (target: ?)
risk → ME = 205 us          (target: ?)
risk_out → gateway_out = 666 us  (target: < 100 us with handler-wake fix)
```

Re-capture script + expected output should diff cleanly
against this file.
