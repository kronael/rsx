# Building a Perpetuals Exchange in Rust

## What RSX Is

RSX is a spec-first perpetuals exchange. 50+ specification
files written before any code. Separate processes -- Gateway,
Risk, Matching Engine, Marketdata, Mark Price, Recorder --
communicating over CMP (C structs over UDP) on the hot path
and WAL replication over TCP on the cold path.

The design budget: under 50 microseconds from gateway ingress
to gateway egress, under 500 nanoseconds for a match inside
the matching engine. The match-engine number is measured
(54 ns single fill, Criterion); the end-to-end number is a
component-sum budget — the continuous E2E harness that
asserts it is the next item on the punch list, not yet
landed. See README "What's measured vs what's a budget."

## Architecture in 60 Seconds

```
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS + CMP bridge
                    | (monoio)   |  JWT, rate limit
                    +-----+------+
                          | CMP/UDP
                    +-----v------+            +----------+
                    |   Risk     |  CMP/UDP   | Matching |
                    |  Engine    +----------->| Engine   |
                    | (1 shard)  |<-----------+ (1/sym)  |
                    +--+---+--+-+  CMP fills  +----+-----+
                       |   |  |                    |
              +--------+   |  +------+        +----+----+
              v            v         v        v         v
         +--------+ +--------+ +--------+ +-------+ +-----+
         |Postgres| | Mark   | |Recorder| |Mktdata| | GW  |
         | (write | | Price  | |(daily  | |(shadow| |(fill|
         | behind)| | Agg    | | WAL)   | | book) | | usr)|
         +--------+ +--------+ +--------+ +-------+ +-----+
```

An order arrives via WebSocket at the Gateway. Gateway
validates (JWT, rate limit, tick/lot size), encodes to CMP,
sends UDP to Risk. Risk checks portfolio margin across all
positions for that user, then forwards CMP/UDP to the
Matching Engine for that symbol. ME matches price-time FIFO,
appends fills to WAL, sends CMP/UDP fills back to Risk. Risk
updates positions, writes behind to Postgres, forwards fills
to Gateway. Gateway pushes WS to the user. Done.

Each hop is a single UDP datagram. No broker. No queue. No
serialization framework. C structs on the wire.

## The Numbers

Criterion benchmarks on the orderbook and transport layer:

| Operation              | Latency  |
|------------------------|----------|
| Match single fill      | 54 ns    |
| Insert resting order   | 857 ns   |
| WAL append (in-memory) | 31 ns    |
| WAL flush+fsync 64KB   | 24 us    |
| CMP encode             | 43 ns    |
| CMP decode             | 9 ns     |

End-to-end on loopback (Gateway → ME → Gateway): not yet
asserted by an automated harness. Summing the measured
components (gateway parse, CMP encode, UDP send, risk
pre-trade, match, WAL append, reverse path) puts the
budget comfortably inside the 50 µs design target, but
treat that as a budget until the harness in
`specs/2/22-perf-verification.md §4` lands.

The matching-engine target was 500 nanoseconds. A single
fill completes in 54 nanoseconds — about 9× under budget,
single thread, no contention (`rsx-book/benches/book_bench.rs`).

## Key Design Decisions

**Fixed-point i64, no floats.** All prices and quantities are
i64 in smallest units. `Price(pub i64)`, `Qty(pub i64)` as
`#[repr(transparent)]` newtypes. Conversion happens once at
the API boundary. No IEEE 754 rounding, deterministic across
architectures, no precision loss across the entire pipeline.

**CMP/UDP, not Kafka.** A message broker adds milliseconds of
latency and an operational dependency. CMP is a custom
protocol inspired by Aeron: C structs over UDP, NACK-based
flow control, per-stream ordering. Each datagram is one
message. No framing, no length prefix, no deserialization.

**Slab arena allocation.** The matching engine pre-allocates
all order slots at startup. O(1) alloc via free list, O(1)
free. Zero calls to malloc on the hot path. 128-byte order
slots aligned to cache lines.

**Single-threaded per symbol.** One matching engine instance
per symbol, pinned to a dedicated core. No locks, no atomic
operations, no MESI cache invalidation. The event loop is a
bare busy-spin -- no `spin_loop()`, no yield.

**SIGTERM = crash.** There is no graceful shutdown path. Every
restart exercises the same WAL replay recovery. This means the
recovery path is tested on every deployment, not just during
incidents.

## The Orderbook

The core data structure is a compressed price level array with
a slab-backed order list at each level.

**CompressionMap.** A perpetuals orderbook might span 20
million tick levels (e.g., BTC from $1 to $200,000 at $0.01
ticks). Allocating 20M array slots wastes memory -- most are
empty. CompressionMap uses 5 distance-based zones around the
current mid-price:

- Zone 0: 1:1 mapping near mid (every tick has a slot)
- Zone 1-3: increasing compression ratios
- Zone 4: catch-all at 50%+ distance from mid

Result: 617K slots (~15MB per side) instead of 20M (~480MB).
Price-to-index lookup is a 2-3 comparison bisection, about
2-5 nanoseconds.

**Slab arena.** Orders live in a pre-allocated `Vec<OrderSlot>`
with a free list. OrderSlot is 128 bytes, `#[repr(C,
align(64))]`, hot fields packed into the first cache line.
Each price level is a doubly-linked list threaded through the
slab. Insert: O(1) append to tail. Cancel: O(1) unlink.

Matching is price-time FIFO. Walk the best price level,
fill orders front to back, advance to next level if needed.

## WAL = Wire = Stream

The WAL disk format, the CMP wire format, and the DXS stream
format are identical. No transformation between them.

Each record: 16-byte header (stream_id, seq, record_type,
payload_len) followed by a `#[repr(C, align(64))]` payload.
The same bytes written to disk are the same bytes sent over
UDP and the same bytes streamed to consumers over TCP.

DXS (the streaming layer) is brokerless. Each producer IS the
replay server. Consumers connect directly to the matching
engine's DXS port and request replay from sequence N. No
central broker, no topic partitions, no consumer groups. The
WAL IS the log.

WalWriter flushes every 10 milliseconds, rotates files at
64MB, retains 10 minutes. Backpressure: if the buffer fills
or flush lag exceeds 10ms, the producer stalls. This is
deliberate -- the matching engine waits rather than dropping
events.

## What We Built

12 Rust crates, roughly 21,000 lines of Rust. 878 Rust tests
passing, 0 failing, in `cargo test --workspace`. ~930 Python
tests in the playground; 421 of 424 Playwright browser tests
passing (3 conditional skips on optional gates).

The twelfth crate is `rsx-messages` — exchange-specific wire
records (Fill, BBO, Order*) that used to live inside the
transport. Splitting them out leaves `rsx-dxs` as a
domain-agnostic transport with zero `rsx-types` dependency in
production builds. The "anyone could use this transport" claim
is now provable rather than aspirational.

All 12 crates build and run end-to-end. The spec corpus is
catching up to the code — see PROGRESS.md for current status
and `.ship/12-SHOWCASE-HONEST/` for the in-flight tightening
work.

A Python/FastAPI playground dashboard with 14 tabs and 60+
API endpoints: process control, order submission, WAL
inspection, fault injection, invariant verification, stress
testing. A React trade UI with orderbook visualization, depth
chart, order entry, positions, and funding history.

A CLI tool that dumps WAL files to JSON with filters by
record type, symbol, user, and time range. Stats mode for
aggregate counts. Follow mode for tailing live WAL writes.

A market maker bot that places two-sided quotes through the
gateway WebSocket with configurable spread, levels, and
refresh interval.

## Recent Work

**Scenarios.** Four deployment scenarios ship with the
playground: minimal (gateway + one ME), duo (two symbols),
full (all processes), stress (full + load generator). Each
scenario is a JSON task list that the playground orchestrates.

**CLI.** The WAL dump tool gained filter flags (--type,
--symbol, --user, --from-ts, --to-ts), --stats for aggregate
counts, --follow for tail mode, and --tick-size/--lot-size
for human-readable price display.

**Bench gate.** A regression gate script runs Criterion
benchmarks and fails if any operation regresses more than 10%
from the stored baseline. Runs in CI.

**Sim cleanup.** The simulator crate was split: fake matching
engine code deleted (the real ME exists now), real WebSocket
stress generator kept as stress.py in the playground.

**Refine pass.** 28 commits across A/B/C/D rounds applying
the project's own wisdom rules uniformly: one import per
line, `.expect("INVARIANT: ...")` instead of bare `unwrap`,
compile-time size/align asserts on every CMP record,
narration comments deleted. The codebase now reads the same
way in every crate.

**Named invariants.** The 10 system-wide correctness
invariants in CLAUDE.md (fills-precede-ORDER_DONE, FIFO
within price level, position = sum of fills, slab no-leak,
funding zero-sum, …) each carry a comment in code naming the
invariant they enforce. `specs/2/6-consistency.md` is the
cross-reference. Audit-by-grep works now.

**Honesty pass.** A skeptical-reviewer audit (`.ship/13-A16Z-FIXES/`)
produced a finding-by-finding resolution map. Things landed:
JWT min-secret + `nbf` + `jti` replay tracker, gateway IP-limiter
cap, zero-heap `send_ring`, WAL append errors propagated, wire
schema version byte. One finding was **rejected on review** —
the CMP source-IP filter contradicted the spec's documented
trust model (CMP is intentionally unauthenticated; auth lives
at the gateway and at L3). A new "Trust boundaries" rule in
CLAUDE.md prevents the same misclassification next time.

## What's Next

Trade UI needs work: nginx WebSocket proxy configuration,
position display, reconnect logic. After that, production
hardening -- the exchange runs, the specs are implemented,
the tests pass. What remains is operational maturity.
