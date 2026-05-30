# RSX

> *Shall I compare thee to a sane design?*
> *Thou art more wondrous and more wild by far.*
> *I fell for thee the night I saw thy spine—*
> *Each SPSC ring, a whisper through a jar.*
>
> *Thy slab did catch me: firm, pre-allocated,*
> *No malloc on thy hot path — O! how pure.*
> *Thy fills, like kisses, never once belated,*
> *Flushed warm to WAL in ten ms, ever sure.*
>
> *The world said "Mad! No mortal weds this thing!"*
> *Yet here I am, pinned to thy dedicated core,*
> *Thy fixed-point heart refusing still to swing—*
> *Each nanosecond makes me love thee more.*
>
> *If thou hast never built it, thou can'st never tell:*
> *The thing impossible may work quite well.*

A perpetual-futures exchange written in Rust. **This is an
educational and research project — a serious one**, built to
eventually grow into a solid, usable system at v1. Today it's an open study artifact you can run
and read. The next step is to deploy it to serve as an
exchange for a small set of esoteric special derivatives,
once the gaps in the "what's not done" list are closed. The
product surface that runs on top of it is sketched at
[krons.fiu.wtf/pub/krons/sfdx](https://krons.fiu.wtf/pub/krons/sfdx/)
— find it if you look.

The transport layer ([rsx-cast](rsx-cast/README.md)) is
domain-agnostic: `cargo tree -p rsx-cast --edges normal | grep
rsx-` is empty. It's the load-bearing piece that proves the
exchange's wire format doubles as on-disk WAL and as the TCP
replay protocol — the same bytes, three uses, no serialization
step.

## Why you'd read this code

You're curious how a working exchange is wired end-to-end,
without the closed-source vendor handwaving. Specs are
written before the code (47 of them in `specs/2/`), every
non-obvious choice has a tradeoff note, and gaps are
labelled gaps rather than dressed up as ship-ready. The
production code is in there; so is the budget table that
says where it falls short and why.

## How fast (measured)

| Layer | p50 | Bench |
|---|---:|---|
| Match algorithm only (dedup + WAL prep + match) | **340 ns** | `rsx-book::matching_bench` |
| In-process round-trip (cast + Orderbook + WAL) | **9.58 µs** | `rsx-cast::cast_rtt_bench` |
| Cross-process production GW→ME→GW | **1 128 µs** | `bench-match-rt` |

99% of the in-process round-trip is the `sendto` syscall;
framing + algorithm together are <0.7%. The 22× gap from
in-process to cross-process is dominated by `monoio::time::sleep(100µs)`
polls in two cast loops (~655 µs alone) plus tokio scheduling.
Where the time goes:
[facts/syscall-latency.md](facts/syscall-latency.md). Bench
catalogue: [docs/benches.md](docs/benches.md).

## What's interesting in the design

- **Casting, the C Message Protocol.** Fixed-size `repr(C)`
  WAL records over UDP between Gateway, Risk, and ME. One
  wire format for disk, network, and memory. NAK + idle-only
  heartbeats for loss recovery; no flow control — slow
  consumers recover via TCP replication, not by stalling the
  producer. Byte layout, comparison vs Aeron / KCP / QUIC,
  and known limits: [specs/2/4-cast.md](specs/2/4-cast.md).
- **Tile architecture.** Pinned threads + rtrb SPSC rings
  where it pays off (full tile arrangement in `rsx-risk`,
  one core-pinned loop in `rsx-matching`); monoio io_uring
  async where I/O multiplexing dominates (`rsx-gateway`,
  `rsx-marketdata`). Per-process split:
  [specs/2/45-tiles.md](specs/2/45-tiles.md).
- **WAL = wire = stream.** The same `repr(C)` bytes go to
  disk, over UDP, and over TCP for replay. No serialization
  step. The 16-byte header carries a `version: u8` at byte 0
  so receivers gate on it before interpreting any other
  field. See [specs/2/48-wal.md](specs/2/48-wal.md) and
  [specs/2/10-replication.md](specs/2/10-replication.md).
- **Slab + CompressionMap orderbook.** Pre-allocated 65 536
  `OrderSlot`s per symbol, sparse-to-dense price compression
  via five distance-based zones (1:1 near-mid, up to 1000:1
  far prices). Zero malloc on the order hot path (GW→Risk→ME→Risk→GW);
  risk margin check iterates positions via a zero-alloc index iterator.
  Off-path operations (BBO scan, funding settlement) allocate small
  per-call Vecs.
  [specs/2/21-orderbook.md](specs/2/21-orderbook.md).

## Architecture

```
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS + cast bridge
                    | (monoio)   |  JWT, rate-limit
                    +-----+------+
                          | casting/UDP
                    +-----v------+            +----------+
                    |   Risk     |  casting/UDP   | Matching |
                    | (1 pinned  +----------->| (1 pinned|
                    |  thread,   |<-----------+  thread, |
                    |  7 rings)  |  cast fills | per-sym) |
                    +--+---+--+-+              +----+-----+
                       |   |  |                     |
              +--------+   |  +------+        +-----v-----+
              v            v         v        v           v
         +--------+ +--------+ +--------+ +-------+ +-------+
         |Postgres| | Mark   | |Recorder| |Mktdata| | Trade |
         | (write | |(monoio | |(daily  | |(monoio| | UI    |
         | behind)| | HTTP)  | | WAL    | |  WS)  | |(React)|
         +--------+ +--------+ +--------+ +-------+ +-------+
```

Three transports:
- **Hot path** between processes: cast/UDP (NAK gap
  recovery, idle-only heartbeats, no flow control).
- **Cold path** between processes: WAL replication over TCP,
  optional rustls TLS. Same record bytes as the WAL.
- **Within a tile-architected process**: rtrb SPSC rings,
  50–170 ns per hop.

Public API to the world is WebSocket JSON (`rsx-gateway`).

## Reading the code

```
rsx-types/      Price, Qty, Side, SymbolConfig newtypes
rsx-cast/       Transport: WAL + casting/UDP + replication/TCP
                (zero rsx-types prod dep)
rsx-messages/   Exchange wire records: Fill, BBO, Order*,
                MarkPrice, Liquidation, ConfigApplied
rsx-book/       Orderbook (Slab, CompressionMap, PriceLevel)
rsx-matching/   ME: per-symbol, single-threaded, core-pinned
rsx-risk/       Risk: per-shard tile
rsx-gateway/    monoio WS + cast bridge, hardened JWT
rsx-marketdata/ monoio shadow book, L2/BBO fan-out
rsx-mark/       External mark-price feeds → cast to risk
rsx-recorder/   Archival replication consumer
rsx-cli/        WAL dump/inspect tool
rsx-log/        Per-thread SPSC → drain → tracing
```

Non-cargo subprojects: `rsx-playground/` (Python/FastAPI dev
dashboard + Playwright), `rsx-webui/` (Vite + React + Tailwind
Trade UI), `rsx-auth/` (Python OAuth service).

## Running it

**Prerequisites:**

| Tool | Why |
|---|---|
| Linux 5.6+ | io_uring (gateway, marketdata) |
| Rust stable | `cargo build --workspace` |
| Postgres 14+ | Risk write-behind, accounts |
| Python 3.14+ | `rsx-playground` (managed via `uv`) |
| `uv`, `bun` | Python deps, webui build + Playwright |

Postgres dev defaults: role `rsx`, db `rsx`, password `rsx`.
Override with `PG_URL=…`. Copy `.env.example` → `.env` for
the full set. Playground mints dev JWTs locally — `rsx-auth`
not needed to demo.

```bash
git clone <repo-url> && cd rsx
make prepare                              # one-time: venv + Playwright
./rsx-playground/playground start         # FastAPI on :49171
# visit http://localhost:49171, click "Start All",
# submit orders, view fills, inspect WAL
./rsx-playground/playground stop
```

60-second clean-boot path: [docs/DEMO.md](docs/DEMO.md).

### Core layout (6-core host)

Hot-path processes busy-spin, so each **must** own a core. An
unpinned spinner floats across cores (CFS load-balancing), lands
on a hot core, starves that consumer, and its UDP socket overflows
→ kernel `RcvbufErrors` → dropped packets → FAULTED replay storm.
This bit us: `rsx-mark` was an unpinned busy-spinner stealing the
gateway's core. Pinning (and making mark ergonomic) fixed it.

| Core | Process | Model |
|---|---|---|
| 0 | OS + mark + recorder | off-path, ergonomic (sleep, ~0% CPU), unpinned |
| 1 | gateway | hot path, pinned, monoio |
| 2 | risk shard 0 | hot path, pinned, busy-spin |
| 3 | ME / matching | hot path, pinned, busy-spin |
| 4 | marketdata | hot path, pinned, monoio |
| 5 | (spare headroom) | — |

Pinning is wired in `start` (`CORE_GW`/`CORE_RISK`/`CORE_ME_0`/
`CORE_MD`). Off-path services (`mark`, `recorder`) sleep instead
of spinning and stay unpinned. On a host with fewer cores, edit
those constants or expect contention.

**From source:**

```bash
make check        # cargo check (fastest feedback)
make test         # Rust unit + integration (~5s)
make e2e          # Rust + API + Playwright (~3 min)
make perf         # Criterion benches
make lint         # clippy -D warnings
```

Single crate: `cargo test -p rsx-book`. Single test:
`cargo test -p rsx-book -- test_name`.

## What's measured vs what's a budget

The microbench numbers above are real and reproducible. The
cross-process production budgets aren't.

| Number | Measured? |
|---|---|
| 54 ns match single fill | yes — `rsx-book` |
| 31 ns WAL buffer append (pre-flush; `Vec` extend) | yes — `rsx-cast` |
| 43 ns Nak/Heartbeat encode, 23 ns Fill encode, 9 ns decode | yes — `rsx-cast` + `rsx-messages` |
| 50–170 ns SPSC ring hop | yes — `rsx-book` |
| <50 µs GW→ME→GW round trip | **design budget**; harness plan in [specs/2/22-perf-verification.md](specs/2/22-perf-verification.md) |
| <500 ns ME match | yes — sub-bench of 54 ns |
| <5 µs risk pre-trade | **design budget** |

## What's not done

The gaps a careful reader will hit:

- **End-to-end latency harness.** The 50 µs / 500 ns numbers
  are budgets, not measurements. Plan in
  [specs/2/22-perf-verification.md](specs/2/22-perf-verification.md).
- **JWT replay protection — long-window.** `JtiTracker` is
  wired into the WS handshake (`rsx-gateway/src/ws.rs`) and
  rejects replayed jti within the last 16 384 tokens (in-memory
  FIFO). A determined attacker who can mint that many fresh
  tokens faster than the legitimate jti is rotated could still
  evict it; long-window dedup needs a TTL ring or persistent
  table.
- **`rsx-cast` UDP** uses `std::net::UdpSocket`, not monoio
  io_uring. One syscall per `sendto`/`recvfrom`. The
  io_uring move is gated on gateway/marketdata owning the
  socket (zero-runtime-dep invariant in `rsx-cast` is
  load-bearing — see `rsx-cast/CLAUDE.md`).
- **Per-consumer FAULTED recovery.** `rsx-matching` has a
  POC `CastRecv::Faulted` → replication-replay path; risk,
  marketdata, and gateway still panic with a pointer to the
  reference impl.
- **`rsx-mark`/`rsx-marketdata` replay** still uses tokio
  for the replication client.

## Specs

47 spec files in [`specs/2/`](specs/2/). The index is
[`specs/index.md`](specs/index.md). If you read three to
start: [1-architecture.md](specs/2/1-architecture.md),
[4-cast.md](specs/2/4-cast.md),
[21-orderbook.md](specs/2/21-orderbook.md).

## Other documents

| | |
|---|---|
| [PROGRESS.md](PROGRESS.md) | per-crate status |
| [GUARANTEES.md](GUARANTEES.md) | consistency, durability, ordering |
| [BLOG.md](BLOG.md) | long-form narrative |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) | failure mode catalogue |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) | operator recovery procedures |
| [TESTING.md](TESTING.md) | test taxonomy |
