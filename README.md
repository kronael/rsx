# RSX

RSX is an educational derivatives exchange you can run locally and
study end to end: gateway, risk, matching, marketdata, WAL, terminal,
and a live Playground dashboard.

The point is not another toy matching engine. The point is to show how
a fast exchange is wired as a whole system: single-core matching,
per-user risk shards, SPSC rings, fixed-point types, WAL-backed
replay, and a browser demo that lets you watch the processes, order
book, fills, logs, and invariants move together.

**Main differentiator:** RSX is both fast and teachable. The hot
components are built around explicit latency budgets and low-level
mechanical choices, while the repo includes specs, notes, benchmark
tables, and an interactive Playground so you can inspect why each
piece exists.

## Run the live demo

```bash
make prepare
./rsx-playground/playground demo minimal
```

Then open [http://localhost:49171](http://localhost:49171).

What to try first:

1. **Overview** — confirm the local RSX processes are running.
2. **Book** — watch the PENGU order book and live fills.
3. **Orders** — submit a test order and trace its lifecycle.
4. **Terminal** — open the embedded `rsx-term` trading terminal.
5. **Verify** — run invariant checks against the live system.

Stop it when done:

```bash
./rsx-playground/playground stop-all
./rsx-playground/playground stop
```

See [docs/demo.md](docs/demo.md) for the 60-second clean-boot path and
[rsx-playground/README.md](rsx-playground/README.md) for the
Playground manual.

## What this is

RSX is a derivatives exchange written in Rust. **This is an
educational and research project — a serious one**, built to
eventually grow into a solid, usable system at v1. Today it is an open
study artifact you can run and read. The next step is to deploy it to
serve as an exchange for a small set of esoteric special derivatives,
once the gaps in the "what's not done" list are closed. The product
surface that runs on top of it is sketched at
[krons.fiu.wtf/pub/krons/sfdx](https://krons.fiu.wtf/pub/krons/sfdx/).

**Instruments:**

- **Perpetuals** — supported. The matching, risk, funding, and
  mark-price paths run today.
- **Options** — not yet.
- **SFDX** (the special derivatives sketched at the link above) —
  not yet; next.

## Why study this code

The point of RSX is to learn how to write **fast distributed
code** — processing millions of events per second with a
target of **sub-10 µs latency over the network**. That target
is aspirational today: the in-process round-trip floor is
7.8 µs p50 / 22.3 µs p99, but cross-process it is still
1.1 ms — the benchmark tables below show exactly where the
time goes and why. Specs are
written before the code — 47 of them in `specs/2/` — and every
non-obvious choice carries a tradeoff note in the crate's
`notes/`. Read the specs for intent, the code for what runs,
and the benchmark tables for the numbers.

Each component below is done and worth studying on its own;
together they show how an exchange is wired end-to-end.

## Components worth studying

Each is a separate crate or process, built and tested (per-crate
status in PROGRESS.md). Maturity: **finalized** — API frozen,
bugfixes only, safe to build on; **release candidate** — benched,
demoed, and settling; **in development** — working but still being
reshaped. Libraries first, then the processes that run on them —
each with the problem it solves.

**Libraries**

- **`rsx-book` — slab + CompressionMap orderbook.** *Release
  candidate.* The problem: keep a limit-order book fast when it
  already holds millions of resting orders — the naive structures
  slow down as they fill. 65 536 pre-allocated `OrderSlot`s per
  symbol and a sparse-to-dense price compression (five distance
  zones, 1:1 near mid up to 1000:1 far) fit a 20M-level book in
  ~20 MB, match in ~30 ns at any depth, and look a price up in
  2–5 ns. [specs/2/21-orderbook.md](specs/2/21-orderbook.md),
  [rsx-book/README.md](rsx-book/README.md).
- **`rsx-cast` — log-backed reliable UDP transport.**
  *Finalized.* The problem: reliable, low-latency messaging
  without a broker. The trick: the wire bytes, the on-disk WAL
  bytes, and the TCP replay-stream bytes are *the same bytes* —
  `repr(C)` records, no serialization step. Domain-agnostic
  (`cargo tree -p rsx-cast --edges normal | grep rsx-` is empty),
  so any project wanting 50-µs-class messaging without Kafka can
  lift it; NAK gap-recovery + idle heartbeats let a slow consumer
  recover via TCP replay instead of stalling the producer.
  [specs/2/4-cast.md](specs/2/4-cast.md),
  [rsx-cast/README.md](rsx-cast/README.md).

**Processes**

- **`rsx-matching` — the matching engine.** *Release candidate.*
  The problem: pair orders with strict price-time priority as fast
  as a single core can go (a symbol is one market — its book can't
  be sharded, so the match itself is the ceiling). One process per
  symbol, single-threaded, core-pinned, bare busy-spin, zero heap
  on the hot path — the match itself flat in book depth (~30 ns at
  1 or 100k resting), and 266 ns for the full accept path (dedup →
  match → WAL → fan-out, one fill that leaves the BBO unchanged) at
  ~3.6M orders/s. No allocation, no locks, no async
  runtime. [specs/2/21-orderbook.md](specs/2/21-orderbook.md),
  [rsx-matching/README.md](rsx-matching/README.md).
- **`rsx-risk` — per-user-shard risk engine.** *Release
  candidate.* The problem: keep solvency-critical margin state in
  RAM on the order critical path while still persisting it durably.
  One core-pinned busy-spin loop with SPSC rings and a tokio
  sidecar for Postgres write-behind *off* the hot path; the
  cross-margin check iterates positions via a zero-alloc index
  iterator. [specs/2/28-risk.md](specs/2/28-risk.md).
- **`rsx-gateway` — WebSocket ingress + cast bridge.** *In
  development.* The problem: absorb many concurrent client
  connections without slowing the hot path. monoio/io_uring for
  many WS fds, a hardened JWT handshake, then a bridge onto the
  cast/UDP hot path — where async I/O multiplexing belongs versus
  where a pinned loop does.
  [specs/2/20-network.md](specs/2/20-network.md).
- **`rsx-marketdata` — shadow book + fan-out.** *In development.*
  The problem: fan market data out to the public without ever
  back-pressuring matching. monoio, off the order critical path;
  drains ME's casting firehose and fans L2 / BBO / trades to
  subscribers, where a slow consumer must never stall the book.
  [specs/2/16-marketdata.md](specs/2/16-marketdata.md).

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

## How it scales

An order from user **U** on symbol **S** routes
`GW → Risk[U] → ME[S] → Risk[U] → GW`. The two shard axes are
independent: add symbols → add ME instances; add users → add
Risk shards. The gateway is stateless and routes by both keys.

**What scales as what:**

- **Gateway** — stateless; scale by **connection count** (add
  instances behind a load balancer). Holds no positions.
- **Risk** — shard by **`user_id`**, using *virtual shards* so
  growth never reshuffles everyone. Two levels of mapping: a user
  hashes to one of a **fixed** number of virtual shards
  (`vshard = hash(user_id) % N_VSHARDS`), and a small **`shardmap`**
  assigns each vshard to a node. A node owns a set of vshards and
  keeps those users' positions + margin in RAM. Because `N_VSHARDS`
  is fixed, adding a node only **reassigns some vshards** to it —
  just those users migrate (warm-catchup + cutover, reusing the
  warm-standby path). A plain `user_id % node_count` would instead
  remap *every* user the instant the node count changes. See
  [specs/2/28-risk.md](specs/2/28-risk.md) §Sharding & scale-out.
- **Matching** — shard by **`symbol_id`**. One pinned engine per
  tradeable instrument, no cross-symbol shared state. More symbols
  → more ME instances.
- **Marketdata** — scale by **public subscriber count**; off the
  order critical path, fan-out only.
- The user axis and the symbol axis are **orthogonal** — grow
  either without touching the other.

The picture:

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
│Mktdata ││ Mark   ││Recorder││Postgres││Terminal│
│L2/BBO  ││ feeds  ││ archive││(write- ││rsx-term│
│fan-out ││ → cast ││  WAL   ││ behind)││        │
│monoio  ││        ││        ││ per    ││        │
│ scale  ││        ││        ││ Risk   ││        │
│by subs ││        ││        ││ shard  ││        │
└────────┘└────────┘└────────┘└────────┘└────────┘
```

Three transports tie it together:
- **Hot path** between processes: cast/UDP (NAK gap recovery,
  idle-only heartbeats, no flow control).
- **Cold path** between processes: WAL replication over TCP,
  optional rustls TLS. Same record bytes as the WAL.
- **Within a tile-architected process**: rtrb SPSC rings,
  50–170 ns per hop.

Public API to the world is WebSocket JSON (`rsx-gateway`). The
`rsx-term` terminal speaks to the gateway and marketdata WebSockets;
the Playground embeds it locally through an xterm.js PTY bridge.

## How fast

Headline numbers only. The full per-bench detail, method, and caveats
live in **[docs/benches.md](docs/benches.md)** — that is the one doc to
keep in sync; this table is just the summary.

| Layer | p50 | what it is |
|---|---:|---|
| Orderbook match | ~30 ns | pure match, any book depth (100 → 10M resting) |
| Matching algorithm (dedup + match + WAL) | 266 ns | ME critical section, no transport; one fill, BBO unchanged |
| In-process round-trip (`bench-match-rt`) | 7.82 µs (22.3 µs p99) | real casting + Orderbook + WAL, one box, no process boundary — the algorithmic floor |
| Cross-process production (GW→ME→GW) | ~1.1 ms | separate processes, end to end |
| **Target: <50 µs GW→ME→GW** | — | **aspirational** |

The gap between the 7.82 µs in-process floor and the ~1.1 ms cross-process
path is the whole story: **~99% of production latency is inter-process
overhead** (monoio sleep, tokio schedule, syscalls), not the match (~30 ns
at any depth) or the transport framing (tens of ns). The io_uring/SQPOLL
work on the roadmap targets exactly that gap. Per-stage budgets — Risk
pre-trade <5 µs, ME match <500 ns — are met inside the components.

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
rsx-term/       Go Bubble Tea trading terminal (trade surface)
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
./rsx-playground/playground demo minimal  # starts Playground + RSX
# visit http://localhost:49171 to inspect processes, books, orders, WAL
./rsx-playground/playground stop-all
./rsx-playground/playground stop
```

60-second clean-boot path: [docs/demo.md](docs/demo.md).

### Core layout (6-core host)

The local Playground demo is laptop-safe by default and does not pin
processes to cores. Core pinning is reserved for explicit performance
or lab runs where the operator controls isolated cores.

| Core | Process | Model |
|---|---|---|
| 0 | OS + mark + recorder | off-path, ergonomic (sleep, ~0% CPU), unpinned |
| 1 | gateway | hot path, pinned, monoio |
| 2 | risk shard 0 | hot path, pinned, busy-spin |
| 3 | ME / matching | hot path, pinned, busy-spin |
| 4 | marketdata | hot path, pinned, monoio |
| 5 | (spare headroom) | — |

Optional pinning is wired in the Playground runtime plan
(`CORE_GW`/`CORE_RISK`/`CORE_ME_0`/`CORE_MD`) and enabled only for
explicit lab/perf runs. Off-path services (`mark`, `recorder`) sleep
instead of spinning and stay unpinned.

Dedicated cores are a **latency** requirement, not a correctness
one. On a small host you can share a core or skip pinning
entirely — everything still works, but the scheduler time-slices
the busy-spinners into **millisecond** round-trips instead of
microseconds: fine for functional tests, meaningless for
benchmarks.

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

`make build` / `cargo build` use the **cranelift** codegen backend
(~7× faster codegen than LLVM on heavy crates); `make release` uses LLVM.
Wired in `.cargo/config.toml` + `rust-toolchain.toml`.

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
- **Userspace UDP is not done — and should be.** `rsx-cast`
  UDP uses `std::net::UdpSocket`, one syscall per
  `sendto`/`recvfrom`; the kernel-bypass move (io_uring, later
  AF_XDP/DPDK) that would cut that syscall is not implemented.
  It applies to every socket-owning caller — matching's ME
  hot-path `recvfrom`/`sendto`, and the gateway/marketdata edges.
  The zero-runtime-dep invariant in `rsx-cast` is load-bearing
  (see `rsx-cast/CLAUDE.md`), so io_uring lives in the *caller*,
  not the transport. The blocker: `CastSender`/`CastReceiver`
  currently own the socket and couple framing with `recv`/`send`.
  The fix is two *additive* cast APIs (expose a built `Framed`'s
  bytes for the caller to io_uring-send; a parse-already-received-
  bytes entry for recv) so the caller owns the socket — a
  sanctioned frozen-cast extension, needing founder sign-off, not
  a redesign.
- **Per-consumer FAULTED recovery.** `rsx-matching` has a
  POC `CastRecv::Faulted` → replication-replay path; risk,
  marketdata, and gateway still panic with a pointer to the
  reference impl.
- **`rsx-mark`/`rsx-marketdata` replay** still uses tokio
  for the replication client.

## Roadmap

The order things get finalized and shipped:

1. **`rsx-book`** — the orderbook library.
2. **`rsx-matching`** — the matching engine.
3. **`rsx-gateway` + `rsx-marketdata`** — finalize the I/O edge.
   The notify egress wake landed; next is the userspace-UDP work
   above (io_uring at the caller, `SQPOLL` gated on the dedicated-
   core config, `SO_REUSEPORT` sharding for capacity). Also scoped
   to this phase, each behind spec work first, not a bounded change:
   the direct **ME→GW fill path** with async settle to Risk (blocked
   on spec gaps — see `BUGS.md` `ME-GW-DIRECT-SPEC-GAPS`), and moving
   ME/Risk **config polling off Postgres to a dedicated config
   server** (`specs/2/57-config-server.md`).
4. **`rsx-term`** — the Go terminal trade surface, including the
   Playground-embedded local view.
5. **Hosting** — complete the deploy so you can install and run
   the terminal against a live exchange.
6. **Blog** — the educational write-up, once the above is real.

Libraries first, then the processes on them, then the client,
then the front door.

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
| [docs/concepts/11-glossary.md](docs/concepts/11-glossary.md) | terms (casting, WAL, vshard, tile, BBO…) — one line each + read-more links |
| [blog/](blog/README.md) | engineering posts + the build manual ([vibe-book](blog/29-building-rsx.md)) + [ops cookbook](blog/28-cookbook.md) — *draft/AI-generated for now, the intended educational core, not yet edited* |

## Reference

| | |
|---|---|
| [PROGRESS.md](PROGRESS.md) | per-component status |
| [GUARANTEES.md](GUARANTEES.md) | **what's lost when** — consistency, durability, data-loss bounds |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) | failure-mode catalogue (per-scenario loss matrix) |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) | operator recovery procedures |
| [TESTING.md](TESTING.md) | test taxonomy |
