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

## Why study this code

The point of RSX is to learn how to write **fast distributed
code** — processing millions of events per second with a
target of **sub-10 µs latency over the network**. That target
is aspirational today: the in-process round trip is 9.58 µs,
but cross-process it is still 1.1 ms — the benchmark tables
below show exactly where the time goes and why. The value is
in reading a *working*, *measured*, *honestly-labelled*
attempt: specs are written before the code (47 of them in
`specs/2/`), every non-obvious choice has a tradeoff note, and
gaps are labelled gaps rather than dressed up as ship-ready —
no closed-source vendor handwaving.

Each component below is done and worth studying on its own;
together they show how an exchange is wired end-to-end.

## Components worth studying

Each is a separate process or crate, done and tested. Why each
one repays a read:

- **`rsx-cast` — log-backed reliable UDP transport.** The
  load-bearing trick: the wire bytes, the on-disk WAL bytes,
  and the TCP replay-stream bytes are *the same bytes* —
  `repr(C)` records, no serialization step. Domain-agnostic
  (`cargo tree -p rsx-cast --edges normal | grep rsx-` is
  empty), so any project wanting 50-µs-class messaging without
  Kafka can lift it. NAK gap-recovery + idle-only heartbeats;
  slow consumers recover via TCP replay, never by stalling the
  producer. [specs/2/4-cast.md](specs/2/4-cast.md),
  [rsx-cast/README.md](rsx-cast/README.md).
- **`rsx-matching` — the matching engine.** Single-threaded,
  core-pinned, bare busy-spin, zero heap on the hot path —
  54 ns per single fill. Read it for how a price-time-priority
  book runs with no allocation, no locks, no async runtime.
  [specs/2/21-orderbook.md](specs/2/21-orderbook.md).
- **`rsx-book` — slab + CompressionMap orderbook.** 65 536
  pre-allocated `OrderSlot`s per symbol; sparse-to-dense price
  compression via five distance-based zones (1:1 near mid, up
  to 1000:1 far). How you fit a 20M-level book in ~15 MB and
  look a price up in 2–5 ns.
- **`rsx-risk` — per-user-shard risk tile.** The full tile
  arrangement: one core-pinned busy-spin loop, SPSC rings, and
  a tokio sidecar for Postgres write-behind *off* the hot path.
  The cross-margin check iterates positions via a zero-alloc
  index iterator. How to keep solvency-critical state in RAM on
  the critical path while persisting asynchronously.
  [specs/2/28-risk.md](specs/2/28-risk.md).
- **`rsx-gateway` — WebSocket ingress + cast bridge.** monoio /
  io_uring for many concurrent WS fds, a hardened JWT
  handshake, then a bridge onto the cast/UDP hot path. Where
  async I/O multiplexing belongs vs where a pinned tile does.
  [specs/2/20-network.md](specs/2/20-network.md).
- **`rsx-marketdata` — shadow book + fan-out.** monoio, off the
  order critical path; drains ME's casting firehose and fans
  L2 / BBO / trades out to public subscribers. The keep-up
  problem: a slow consumer here must never back-pressure
  matching. [specs/2/16-marketdata.md](specs/2/16-marketdata.md).

Supporting cast: `rsx-types` (fixed-point newtypes),
`rsx-messages` (the wire records), `rsx-mark` (external feeds →
cast), `rsx-recorder` (archival replay consumer), `rsx-log`
(off-hot-path logging), `rsx-cli` (WAL inspect).

## Architecture

An order from user **U** on symbol **S** routes
`GW → Risk[U] → ME[S] → Risk[U] → GW`. The two shard axes are
independent: add symbols → add ME instances; add users → add
Risk shards. The gateway is stateless and routes by both keys.

```
  many clients (WS, JSON)                       SCALE-OUT AXES
   │    │    │    │                              ──────────────
   ▼    ▼    ▼    ▼
 ┌───────────────────────────┐
 │  Gateway  (monoio)         │ ◀── add instances    STATELESS
 │  WS · JWT · routes U→Risk, │     per connection    front; no
 │  S→ME · cast bridge        │     load              per-user
 └─────────────┬─────────────┘                       state held
   order(user U │ symbol S)   │ casting/UDP
                ▼
 ┌───────────────────────────┐
 │  Risk  —  SHARD BY user_id │ ◀── add shards =      each shard
 │  Risk[0] Risk[1] … Risk[k] │     more users        OWNS a user
 │  positions + margin in RAM │                       range; pinned,
 │  1 pinned busy-spin loop   │                       busy-spin tile
 └──────┬───────────────▲─────┘
 reserve│ (sync)        │settle (async)  casting/UDP
        ▼               │
 ┌───────────────────────────┐
 │  ME  —  SHARD BY symbol    │ ◀── add engines =     one engine
 │  ME[BTC] ME[ETH] … ME[sym] │     more symbols      PER symbol;
 │  single-thread, pinned,    │                       no shared
 │  busy-spin, zero-heap match│                       state across
 └─────────────┬─────────────┘                       symbols
               │ fills / BBO  (fire-and-forget, off the critical path)
  ┌────────┬───┴────┬─────────┬──────────┐
  ▼        ▼        ▼         ▼          ▼
┌────────┐┌────────┐┌────────┐┌────────┐┌────────┐
│Mktdata ││ Mark   ││Recorder││Postgres││Trade UI│
│L2/BBO  ││ feeds  ││ archive││(write- ││(React) │
│fan-out ││ → cast ││  WAL   ││ behind)││        │
│monoio  ││        ││        ││ per    ││        │
│ scale  ││        ││        ││ Risk   ││        │
│by subs ││        ││        ││ shard  ││        │
└────────┘└────────┘└────────┘└────────┘└────────┘
```

**What scales as what:**
- **Gateway** — stateless; scale by **connection count** (add
  instances behind a load balancer). Holds no positions.
- **Risk** — shard by **`user_id`**. Each shard owns a disjoint
  range of users and holds their positions + margin in RAM.
  More users → more shards.
- **ME (matching)** — shard by **`symbol_id`**. One pinned
  engine per tradeable instrument, no cross-symbol shared state.
  More symbols → more ME instances.
- **Marketdata** — scale by **public subscriber count**; off the
  order critical path, fan-out only.
- The user axis and the symbol axis are **orthogonal** — grow
  either without touching the other.

Three transports tie it together:
- **Hot path** between processes: cast/UDP (NAK gap recovery,
  idle-only heartbeats, no flow control).
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

Hot-path processes busy-spin, so each **must** own a dedicated
core. Without pinning they float across cores and starve each
other — a starved hot-path process stalls, so pin each one to
its own core.

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
of spinning and stay unpinned. The hot-path processes need a
dedicated core each — on a host with fewer cores than that, you
can't run them without starving each other.

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
