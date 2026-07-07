# RSX

**Your Lowest-Latency Derivatives exchange with Portfolio Margin.**

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

Each is a separate process or crate, built and tested (per-crate
status in PROGRESS.md). Maturity is marked per component:
**finalized** — API frozen, bugfixes only, safe to build on;
**release candidate** — benched, demoed, and settling, the shape
won't change much; **in development** — working but still being
reshaped. Why each one repays a read:

- **`rsx-cast` — log-backed reliable UDP transport.**
  *Finalized: API frozen, bugfixes only.* The
  load-bearing trick: the wire bytes, the on-disk WAL bytes,
  and the TCP replay-stream bytes are *the same bytes* —
  `repr(C)` records, no serialization step. Domain-agnostic
  (`cargo tree -p rsx-cast --edges normal | grep rsx-` is
  empty), so any project wanting 50-µs-class messaging without
  Kafka can lift it. NAK gap-recovery + idle-only heartbeats;
  slow consumers recover via TCP replay, never by stalling the
  producer. [specs/2/4-cast.md](specs/2/4-cast.md),
  [rsx-cast/README.md](rsx-cast/README.md).
- **`rsx-matching` — the matching engine.** *Release
  candidate.* Single-threaded,
  core-pinned, bare busy-spin, zero heap on the hot path —
  54 ns per single fill. Read it for how a price-time-priority
  book runs with no allocation, no locks, no async runtime.
  [specs/2/21-orderbook.md](specs/2/21-orderbook.md).
- **`rsx-book` — slab + CompressionMap orderbook.** *Release
  candidate.* 65 536
  pre-allocated `OrderSlot`s per symbol; sparse-to-dense price
  compression via five distance-based zones (1:1 near mid, up
  to 1000:1 far). How you fit a 20M-level book in ~15 MB and
  look a price up in 2–5 ns.
- **`rsx-risk` — per-user-shard risk tile.** *Release
  candidate.* The full tile
  arrangement: one core-pinned busy-spin loop, SPSC rings, and
  a tokio sidecar for Postgres write-behind *off* the hot path.
  The cross-margin check iterates positions via a zero-alloc
  index iterator. How to keep solvency-critical state in RAM on
  the critical path while persisting asynchronously.
  [specs/2/28-risk.md](specs/2/28-risk.md).
- **`rsx-gateway` — WebSocket ingress + cast bridge.** *In
  development.* monoio /
  io_uring for many concurrent WS fds, a hardened JWT
  handshake, then a bridge onto the cast/UDP hot path. Where
  async I/O multiplexing belongs vs where a pinned tile does.
  [specs/2/20-network.md](specs/2/20-network.md).
- **`rsx-marketdata` — shadow book + fan-out.** *In
  development.* monoio, off the
  order critical path; drains ME's casting firehose and fans
  L2 / BBO / trades out to public subscribers. The keep-up
  problem: a slow consumer here must never back-pressure
  matching. [specs/2/16-marketdata.md](specs/2/16-marketdata.md).

Supporting cast: `rsx-types` (fixed-point newtypes),
`rsx-messages` (the wire records), `rsx-mark` (external feeds →
cast), `rsx-recorder` (archival replay consumer), `rsx-log`
(off-hot-path logging), `rsx-cli` (WAL inspect).

## Specs vs ARCHITECTURE — intent vs what-is

Everything here starts as a spec ([specs/2/](specs/2/), 47 of
them) — they are referenced throughout the code and docs. A spec
captures the **intent before the implementation**. The design is
fluid: when implementation shows a spec impossible or impractical,
the spec is refined rather than defended. The **ARCHITECTURE
documents** (one at the repo root, one per crate) are the
authoritative record of **how things are now**. Read them in that
order: the spec for why it was designed, ARCHITECTURE for what
actually runs.

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
 └──────────────┬────────────┘                       state held
   order(user U │ symbol S)   │ casting/UDP
                ▼
 ┌───────────────────────────┐
 │  Risk — SHARD BY user_id   │ ◀── add a shard =     owns a set of
 │  via fixed vshards + map   │     move ~1/N of the   vshards (not
 │  Risk[0] Risk[1] … Risk[k] │     vshard slots to it  a live-count
 │  positions + margin in RAM │     (rest stay put)     modulo); pinned
 └──────┬───────────────▲─────┘
 reserve│ (sync)        │settle (async)  casting/UDP
        ▼               │
 ┌──────────────────────┴────┐
 │  Matching — SHARD BY symbol│ ◀── add engines =     one engine
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
- **Risk** — shard by **`user_id`** via *fixed virtual shards + a
  map*: `vshard = hash(user_id) % N_VSHARDS` (N_VSHARDS fixed) →
  `shardmap[vshard]` → node. Each shard owns a set of vshards and
  holds those users' positions + margin in RAM. Adding a shard
  moves only ~1/N of the vshard slots (and migrates those users by
  warm-catchup + cutover, reusing the warm-standby path) — *not* a
  `user_id % shard_count` reshuffle of everyone. See
  [specs/2/28-risk.md](specs/2/28-risk.md) §Sharding & scale-out.
- **Matching** — shard by **`symbol_id`**. One pinned
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

Public API to the world is WebSocket JSON (`rsx-gateway`),
with a protobuf-over-QUIC feed (`rsx-tui`) planned.

## How fast

Measured at commit `7a6846a`, 6-core box, no core isolation, debug
vs release noted. The sub-10 µs *network* target is aspirational — the
in-process floor is there, the cross-process path isn't yet. Full method
+ curves: [reports/20260530_load-curves.md](reports/20260530_load-curves.md).

### (A) Components under load — service time, fastest first

Closed-loop service time (the CPU cost of the work; no queue). Each part
absorbs the offered rate in the left column until it saturates.

| Part | sustained | p50 | p99 |
|---|---:|---:|---:|
| Risk: reject (not-in-shard) | 96 M/s | 30 ns | 40 ns |
| Orderbook: match, touch stays resting (book 100–10M deep) | — | 60-65 ns | — |
| Orderbook: match that CLEARS the touch level | — | 145 ns | — |
| Risk: reject (insufficient margin) | 26 M/s | 50 ns | 90 ns |
| Risk: accept order (0 open/user) | 10.0 M/s | 110 ns | 181 ns |
| ME: full accept (dedup + WAL + match + events + index) | — | 205 ns | — |
| Risk: accept order (64 open/user) | 5.2 M/s | 191 ns | 321 ns |
| Risk: process fill (hot users) | 4.8 M/s | 200 ns | 351 ns |
| Risk: accept order (512 open/user) | 1.1 M/s | 852 ns | 1313 ns |

The orderbook match holds **~60-65 ns p50 whether the book has 100, 1M, or
10M resting orders** — the compressed slab + occupancy bitmap
(`rsx-book/src/occupancy.rs`) make level lookup and next-best-level find
O(depth=3), not O(book size). That "O(1) in depth" claim covers the
match/next-best primitive specifically — matching a marketable order and,
when it clears the touch level, finding the new best level (145 ns, still
depth-invariant). It does NOT cover every op: FOK's fill-or-kill
feasibility check is a separate full-book scan, still O(N) (bugs.md
`FOK-AVAILABLE-LIQUIDITY-ON-SCAN`). Risk accept cost scales with a user's
*open-order* count (the frozen-margin sum), shown by the depth sweep.
**Load curve** (open-loop, no coordinated omission): the risk shard holds
flat **~0.16 µs p50 up to ~4 M orders/s** offered, then knees at **~6 M/s**
(accept ratio → 91%, p99 → 34 ms, backlog unbounded). Under persist
backpressure the fill path **stalls, never drops**.

### (B) Network stack — rsx-cast, loopback, pinned

| Hop | p50 |
|---|---:|
| One-way (`CastSender::send` → `try_recv`) | 3.89 µs |
| Round-trip echo (2 hops) | 7.60 µs |

Loopback only — a real NIC adds IRQ + driver tx/rx. ~99% of this is the
`sendto`/`recvfrom` syscall, which the io_uring move targets (see end).
Wire microbenchmarks (release): WAL append 31 ns, Nak/Heartbeat encode
43 ns, Fill encode 23 ns, decode 9 ns; SPSC ring hop 50–170 ns.

### (C) Whole exchange GW→ME→GW — single stream

| Path | p50 | p99 | note |
|---|---:|---:|---|
| In-process round-trip (cast/UDP loopback + full ME) | 7.5 µs | 16.9 µs | transport-bound floor, measured |
| Live WS single warmed stream | 2.25 ms | 18.8 ms | gateway reactor egress; **−80%** after the egress-drain fix (was 11.5 ms) |
| REST `/health` (fresh conn) | ~115 µs | ~1.4 ms | measured |
| **Target: <50 µs GW→ME→GW** | — | — | **aspirational** |

Risk pre-trade (<5 µs budget) and ME match (<500 ns budget) are now
*genuinely* met in the component numbers above (110 ns accept, 205 ns full
accept, 145 ns even when a match clears the touch level) — before the
occupancy-bitmap fix (`da9a2b4`, 2026-07-04) the <500 ns match budget was
only met on the path that never clears a level; clearing one cost
32-120 µs (bugs.md `MATCHING-BENCH-ORDERTYPE-FIXTURE`, now fixed). The
open gap is the cross-process whole-e2e path. Parallel/flood whole-e2e is
blocked by `ME-FAULTED-NO-REPLAY-ADDR` (bugs.md) — single-stream only.

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
rsx-tui/        ratatui trading terminal (trade surface)
```

Non-cargo subprojects: `rsx-playground/` (Python/FastAPI dev
dashboard + Playwright), `rsx-auth/` (Python OAuth service).

## Running it

**Prerequisites:**

| Tool | Why |
|---|---|
| Linux 5.6+ | io_uring (gateway, marketdata) |
| Rust nightly | pinned in `rust-toolchain.toml`; auto-installs |
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
of spinning and stay unpinned.

The dedicated cores are a **latency** requirement, not a
correctness one. For testing on a small host you can pin several
hot-path processes to the same core (or leave them unpinned) and
everything still works — the scheduler time-slices the
busy-spinners at millisecond quanta, so expect millisecond
round-trips instead of microseconds. Fine for functional tests;
meaningless for benchmarks.

**From source:**

Prerequisites: **nightly** Rust (pinned in `rust-toolchain.toml`; the
cranelift codegen backend and clippy/rustfmt auto-install on first build),
and the **mold** linker —
`.cargo/config.toml` links every crate with `-fuse-ld=mold`
(`sudo apt install mold`, or `brew install mold`). Without it
builds fail at link time with `cannot find 'mold'`; either
install it or delete `.cargo/config.toml` to fall back to the
default linker (slower links, same binaries).

```bash
make check        # cargo check (fastest feedback)
make test         # Rust unit + integration (~5s)
make e2e          # Rust + API + Playwright (~3 min)
make perf         # Criterion benches
make lint         # clippy -D warnings
```

Single crate: `cargo test -p rsx-book`. Single test:
`cargo test -p rsx-book -- test_name`.

Debug builds use the **cranelift** codegen backend (~7× faster codegen on
heavy crates than LLVM); `make build` / `cargo build`. `make release`
(`--release`) uses LLVM, optimized. Configured in `.cargo/config.toml` +
`rust-toolchain.toml` — no flags, nothing to remember.

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

## Concepts & blog

| | |
|---|---|
| [docs/concepts/](docs/concepts/index.md) | the design choices, each explained — *why* it's the right call |
| [docs/concepts/glossary.md](docs/concepts/glossary.md) | terms (casting, WAL, vshard, tile, BBO…) — one line each + read-more links |
| [blog/](blog/README.md) | engineering posts + the build manual ([vibe-book](blog/29-building-rsx.md)) + [ops cookbook](blog/28-cookbook.md) |

## Reference

| | |
|---|---|
| [PROGRESS.md](PROGRESS.md) | per-component status |
| [GUARANTEES.md](GUARANTEES.md) | **what's lost when** — consistency, durability, data-loss bounds |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) | failure-mode catalogue (per-scenario loss matrix) |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) | operator recovery procedures |
| [TESTING.md](TESTING.md) | test taxonomy |

## Next step in the experiment

**`rsx-cast` moves from `std::net::UdpSocket` to io_uring.** Today it
does one `sendto`/`recvfrom` syscall per packet — and on the loopback
benches *that syscall is the dominant cost* (~99% of the in-process round
trip). A single-message loopback bench won't show io_uring as faster:
there's no NIC, no IRQ, and a lone packet has nothing to batch. The win
appears under high packet rates on a real NIC, where io_uring amortizes
submissions across a shared kernel/userspace ring and collapses the
per-packet syscall. The move is gated on gateway/marketdata owning the
socket — the zero-runtime-dep invariant in `rsx-cast` is load-bearing
(see `rsx-cast/CLAUDE.md`). It's the next networking experiment, not a
bug fix.
