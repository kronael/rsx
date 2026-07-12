# Building a Derivatives Exchange in Rust

> **Draft — AI-generated, not yet human-edited (slop, for now).** These
> posts are meant to be the educational core of RSX — the *why* behind the
> code — but they haven't had a real editorial pass yet. For anything
> authoritative, read the specs (`specs/2/`) and the code. The blog gets a
> proper rewrite before it becomes the front door.

## What RSX Is

Two artifacts in one repo:

1. **`rsx-cast` — an open-source, log-backed reliable UDP
   transport.** WAL on disk, casting on the wire, replication for replay.
   The WAL bytes, the wire bytes, and the replay stream bytes
   are the same bytes. Domain-agnostic — `cargo tree -p
   rsx-cast --edges normal | grep rsx-` is empty. Any project
   that wants 50-µs-class messaging without Kafka can use it.

2. **A complete derivatives exchange built on it.** Gateway,
   Risk, Matching Engine, Marketdata, Mark Price, Recorder —
   each a separate process. Spec-first: 47+ spec files
   written before any code. The exchange is both a real
   product and the load-bearing demo that proves `rsx-cast`
   handles a non-trivial workload.

The design budget: under 50 microseconds from gateway ingress
to gateway egress, under 500 nanoseconds for a match inside
the matching engine. The match-engine number is measured
(54 ns single fill, Criterion). The end-to-end number is a
component-sum budget — the continuous probe that asserts it
is now wired (`make perf-load` writes the measured distribution to
`bench-e2e-latest.json`), with the cid-matching fix from
F22 making the first reliable number trustworthy.

## Architecture in 60 Seconds

```
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS + casting bridge
                    | (monoio)   |  JWT, rate limit
                    +-----+------+
                          | casting/UDP
                    +-----v------+            +----------+
                    |   Risk     |  casting/UDP   | Matching |
                    |  Engine    +----------->| Engine   |
                    | (1 shard)  |<-----------+ (1/sym)  |
                    +--+---+--+-+  casting fills  +----+-----+
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
validates (JWT, rate limit, tick/lot size), encodes to casting,
sends UDP to Risk. Risk checks portfolio margin across all
positions for that user, then forwards casting/UDP to the
Matching Engine for that symbol. ME matches price-time FIFO,
appends fills to WAL, sends casting/UDP fills back to Risk. Risk
updates positions, writes behind to Postgres, forwards fills
to Gateway. Gateway pushes WS to the user. Done.

Each hop is a single UDP datagram. No broker. No queue. No
serialization framework. C structs on the wire.

## The Numbers

Criterion benchmarks on the orderbook and transport layer:

| Operation                                      | Latency  |
|------------------------------------------------|----------|
| Match single fill                              | 54 ns    |
| Insert resting order                           | 857 ns   |
| `WalWriter::prepare` + `append_framed` (Vec extend, pre-fsync) | ~38 ns |
| WAL flush + fsync, 1 record (unamortised)      | ~426 us  |
| WAL flush + fsync, amortised at 1 000 rec/flush | ~1.1 us/rec |
| Protocol-record encode (Nak / CastHeartbeat) | 43 ns |
| `FillRecord` encode                            | 23 ns    |
| Protocol-record decode                         | 9 ns     |

End-to-end on loopback (Gateway → ME → Gateway): not yet
asserted by an automated harness. Summing the measured
components (gateway parse, casting encode, UDP send, risk
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

**casting/UDP, not Kafka.** A message broker adds milliseconds of
latency and an operational dependency. casting is a custom
protocol inspired by Aeron: C structs over UDP, NAK-based
gap recovery, per-stream ordering, no congestion control. Each
datagram is one fixed-size record behind a 16-byte header — no
variable-length framing, no length-prefix parsing, no
deserialization step; the receiver casts the bytes straight
into a `#[repr(C)]` struct.

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

The WAL disk format, the casting wire format, and the replication stream
format are identical. No transformation between them.

Each record: 16-byte header (version, record_type, payload
length, CRC32C over the payload) followed by a `#[repr(C,
align(64))]` payload; the sequence number lives in the payload,
not the header. The same bytes written to disk are the same
bytes sent over UDP and the same bytes streamed to consumers
over TCP.

replication (the cold-path layer) is brokerless. Each producer
IS the replay server. Consumers connect directly to the
matching engine's replication port and request replay from
sequence N. No central broker, no topic partitions, no
consumer groups. The WAL IS the log.

WalWriter flushes every 10 milliseconds, rotates files at
64MB, retains 4 hours. Backpressure: if the buffer fills
or flush lag exceeds 10ms, the producer stalls. This is
deliberate -- the matching engine waits rather than dropping
events.

## What We Built

12 Rust crates, roughly 22,000 lines of Rust. **883 Rust
tests passing, 0 failing.** ~930 Python tests in the
playground. **455 Playwright** browser tests across 24 spec
files (up from 421/23 — the audit cleanup added regression
coverage for every finding).

The twelfth crate is `rsx-messages` — exchange-specific wire
records (Fill, BBO, Order*) extracted from the transport so
`rsx-cast` stays domain-agnostic. The "anyone could use this
transport" claim is provable: `cargo tree -p rsx-cast --edges
normal` has no `rsx-` entries.

All 12 crates build and run end-to-end. See PROGRESS.md for
current crate status; the 28-finding dashboard-honesty audit
shipped and the sprint dir was pruned on close-out (findings
baked into the code + CHANGELOG.md).

A Python/FastAPI playground dashboard with 14 tabs and 60+
API endpoints: process control, order submission, WAL
inspection, fault injection, invariant verification, stress
testing. A React trade UI with orderbook visualization, depth
chart, order entry, positions, and funding history.

A CLI tool that dumps WAL files to JSON with filters by
record type, symbol, user, and time range. Stats mode for
aggregate counts. Follow mode for tailing live WAL writes.

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
compile-time size/align asserts on every casting record,
narration comments deleted. The codebase now reads the same
way in every crate.

**Named invariants.** The 10 system-wide correctness
invariants in CLAUDE.md (fills-precede-ORDER_DONE, FIFO
within price level, position = sum of fills, slab no-leak,
funding zero-sum, …) each carry a comment in code naming the
invariant they enforce. `specs/2/6-consistency.md` is the
cross-reference. Audit-by-grep works now.

**Honesty pass.** A skeptical-reviewer audit (see CHANGELOG v0.2.0)
produced a finding-by-finding resolution map. Things landed:
JWT min-secret + `nbf` + `jti` replay tracker (now fully wired
through `ws_handshake`), gateway IP-limiter cap, zero-heap
`send_ring`, WAL append errors propagated, wire schema version
byte. One finding was **rejected on review** — the casting
source-IP filter contradicted the spec's documented trust
model (casting is intentionally unauthenticated; auth lives at
the gateway and at L3). A new "Trust boundaries" rule in
CLAUDE.md prevents the same misclassification next time.

**Dashboard-honesty pass.** A two-step audit (agent-browser
walked the live UI; codex adversarially read the endpoint
handlers) catalogued **28 ways the playground was lying** —
"100 GREEN" while the matching engine was crash-looping, a
"circuit breaker: closed" string literal in the gateway
topology card, `/x/core-affinity` rendering "Core {i}" from a
list index with no `sched_getaffinity` call, a latency probe
that returned on the first fill frame regardless of cid and
then echoed the probe's own cid so it *looked* matched. All
28 are now closed. Where a real data source existed, we
compute correctly. Where one didn't, we removed the panel or
labeled it honestly ("WAL stream lag (proxy)", "synthetic
demo index"). A dashboard that admits ignorance beats one
that performs confidence.

## The Terminal — an Order Book You Can Read

Most of RSX is machinery a human never sees: casting frames, WAL bytes, matching
in tens of nanoseconds. `rsx-term` is the opposite — the one surface a person
actually looks at — and it asks a different question: how does a *human* read
order flow?

The pros' answer is Bookmap: a heavy, closed, GPU heatmap in its own window that
shows resting liquidity over time, so you can watch walls appear and get pulled
(spoofing) or trades pound a level that won't shrink (absorption). We built that
in text. `rsx-term` renders a live liquidity heatmap into a character grid: **time
flows up** on a logarithmic cadence (live → 10 s → minutes → hours), **price** runs
on a fisheye (fine at the touch, compressed into the deep book), and each cell's
colour and glyph carry resting size and order count. Trades overlay as a second
layer. Patterns you'd otherwise infer after the fact become shapes you watch
happen.

The trick that fits a whole book *plus its history* into an 80×40 grid is one idea
applied three times: **replace a linear axis with a nonlinear one that magnifies
what matters and compresses the periphery.** Time is logarithmic (recent fine,
distant coarse); price is a fisheye (touch fine, deep aggregated); size is a log
colour ramp (so one whale doesn't black out the rest). Each keeps full fidelity
where a trader looks and gracefully coarsens the rest — a focus-plus-context lens
on the market. Derivation: `rsx-term/notes/compression.md`.

Two deliberate constraints. There is **no mouse** — in a fast trading surface a
mouse is a fat-finger and latency liability; every action is a keystroke, and a
price cursor on `h`/`l` covers what a click would. And it **never fabricates a
number**: a value with no source shows a dash, an estimate is marked `~`, an order
over the fat-finger cap is hard-blocked (not a dismissable warning), and a P&L
that would overflow i64 is *withheld* rather than shown wrapped. A dash you can
see beats a number you can't trust.

## Measuring Every Glyph

A tangent worth its own entry, because it's the kind of thing you only catch by
looking. The heatmap paints each cell with a character, and the obvious intensity
ramp is the Unicode shade family — space, `░`, `▒`, `▓`, `█` — assumed to be
0/25/50/75/100 % ink. Rasterise them in the actual terminal font, count the black
pixels, and they measure **0, 22, 56, 86, 100** — not evenly spaced; a ramp built
on the assumption bands at the low end. Worse, braille (`U+2800`–`U+28FF`), which
*promises* eight sub-cell pixels for a finer ramp, renders as an identical empty
`.notdef` box for every codepoint in the default monospace font — an invisible
channel that *looks* like it's working.

So the glyph vocabulary is calibrated empirically: rasterise every candidate,
measure its coverage, and order by what the font actually draws — the same trick
ASCII-art renderers use, pointed at a trading heatmap. The measurement sorts
glyphs into roles: shades give four coarse uniform levels (colour carries the fine
gradation), letters fill the light end finely and render *everywhere*, distinct
markers (`●◆■`) carry the trade layer, braille is dropped. The lesson generalises
well past this terminal: **a character is a picture; measure its ink, don't trust
its name.** (`rsx-term/notes/glyph-bank.md`, and the tool that does the measuring,
`rsx-term/tools/glyphbank/`.)

## What's Next

The exchange runs end-to-end. The open questions we're actively
working through: tile-architecture parity for gateway and
marketdata (currently monoio reactors, not pinned cores);
multicast fan-out for casting v2 (one ME → N consumers, no
per-receiver copy); measured GW→ME→GW p50/p99 under sustained
load rather than the current component-sum estimate. The
`rsx-cast` transport layer is already domain-agnostic — the
blog post will include a worked example of a non-exchange
consumer using it.
