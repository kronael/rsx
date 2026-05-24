# RSX

Two artifacts in one repo:

1. **`rsx-dxs` — an open-source, log-backed reliable UDP
   transport.** WAL on disk, CMP on the wire, DXS for replay.
   The WAL bytes, the UDP bytes, and the replay-stream bytes
   are the same bytes. Domain-agnostic: `cargo tree -p rsx-dxs
   --edges normal | grep rsx-` is empty. Drop it into any
   project that needs 50-µs-class messaging without Kafka.
2. **A complete perpetuals exchange built on it.** Gateway,
   Risk, Matching Engine, Marketdata, Mark, Recorder, Maker —
   each a separate process. Spec-first: 47 spec files in
   `specs/2/` written before the code. The exchange is both a
   real product and the load-bearing demo that proves `rsx-dxs`
   handles a non-trivial workload.

The wedge — "open-source the orthogonal libs that already
exist, sell the exchange-in-a-box on top" — is written up in
[specs/2/50-wedge.md](specs/2/50-wedge.md). The one-screen
pitch is in [ONEPAGER.md](ONEPAGER.md). The longer narrative
is in [BLOG.md](BLOG.md).

## How fast

| Layer | p50 |
|---|---:|
| Match algorithm only (dedup + WAL + match) | **340 ns** |
| In-process round-trip (real CMP + Orderbook + WAL) | **9.58 µs** |
| Cross-process production (GW→ME→GW) | **1 128 µs** |

99% of the in-process round-trip is the `sendto` syscall.
Framing + algorithm together are <0.7%. Optimisation paths
(io_uring SQPOLL, sendmmsg, DPDK/AF_XDP) are documented in
[facts/syscall-latency.md](facts/syscall-latency.md) and
[docs/benches.md](docs/benches.md). Design budget for the
production path is **<50 µs**; current p50 is 22× over and
we know why (`monoio::time::sleep(100µs)` in two CMP poll
loops accounts for ~655 µs of it).

## What's interesting here

- **CMP, the C Message Protocol** — fixed-size `repr(C)` WAL
  records over UDP between Gateway, Risk, and ME. One wire
  format for disk, network, and memory. NAK + heartbeat for
  loss recovery, sequence-window flow control. See
  [specs/2/4-cmp.md](specs/2/4-cmp.md) for byte layout,
  comparison vs Aeron / kcp / QUIC, and known limits.
- **Tile architecture** — pinned threads + rtrb SPSC rings
  where it pays off (full tile arrangement in `rsx-risk`),
  monoio io_uring async where I/O multiplexing dominates
  (`rsx-gateway`, `rsx-marketdata`), single core-pinned loop
  for compute (`rsx-matching`). Per-process split in
  [specs/2/45-tiles.md](specs/2/45-tiles.md).
- **WAL = wire = stream** — the same `repr(C)` bytes go to
  disk, over UDP, and over TCP for replay. No serialisation
  step. The 16-byte header carries a `version: u8` at byte 8
  (V0=legacy, V1=current) so future format changes can roll
  out without breaking replay. See
  [specs/2/48-wal.md](specs/2/48-wal.md) and
  [specs/2/10-dxs.md](specs/2/10-dxs.md).
- **Slab + CompressionMap orderbook** — pre-allocated 65 536
  OrderSlots per symbol, sparse-to-dense price compression
  via 5 distance-based zones (1:1 near-mid, up to 1000:1 far
  prices). Zero malloc on the hot path. See
  [specs/2/21-orderbook.md](specs/2/21-orderbook.md).

## Quick start

**Prerequisites** (the part previous READMEs didn't tell you):

| Tool          | Why                                                |
|---------------|----------------------------------------------------|
| Linux 5.6+    | io_uring (gateway, marketdata)                     |
| Rust stable   | `cargo build --workspace` (rust-toolchain.toml)    |
| Postgres 14+  | Risk write-behind, accounts, frozen_orders         |
| Python 3.14+  | rsx-playground server (managed via `uv`)           |
| `uv`          | Python dep manager (https://github.com/astral-sh/uv)|
| `bun`         | rsx-webui Trade UI build + Playwright runner       |

Postgres needs role `rsx`, db `rsx`, password `rsx` for the
default dev URL. Override with `PG_URL=…` if you have a
different setup. The `rsx-auth` service additionally needs
GitHub OAuth credentials, but the playground runs without
the auth service — it mints dev JWTs from
`RSX_GW_JWT_SECRET`. Copy [`.env.example`](.env.example)
to `.env` for the full list of variables.

```bash
git clone <repo-url> && cd rsx

# One-time: prepare Python venv + Playwright browsers
make prepare

# Start the playground (FastAPI dashboard on :49171)
./rsx-playground/playground start

# Visit http://localhost:49171, click "Start All" to launch
# the 7 RSX processes (gateway, risk, ME, marketdata, mark,
# recorder, maker), submit orders, view fills, inspect WAL

# Stop when done
./rsx-playground/playground stop
```

The 60-second clean-boot path is in [docs/DEMO.md](docs/DEMO.md).

**Build and test from source:**

```bash
cargo check                # type check (fastest)
cargo build --workspace    # debug build (~5 min cold)
cargo test --workspace     # 887 passing (unit + integration)
make perf                  # Criterion benches
make bench-gate            # local 10% regression gate
```

## Architecture

```
                    +------------+
                    |  Web (WS)  |
                    +-----+------+
                          |
                    +-----v------+
                    |  Gateway   |  WS + CMP bridge
                    |  (monoio,  |  JWT, rate limit
                    |   async)   |
                    +-----+------+
                          | CMP/UDP
                    +-----v------+            +----------+
                    |   Risk     |  CMP/UDP   | Matching |
                    | (1 pinned  +----------->| (1 pinned|
                    |  thread,   |<-----------+  thread, |
                    |  7 rings)  |  CMP fills | per-sym) |
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

Transports:
- **Hot path** between processes: CMP/UDP (sequence-window
  flow control, NAK gap recovery)
- **Cold path** between processes: WAL replication over TCP
  with optional rustls TLS (replay, replication, archival)
- **Within a tile-architected process**: rtrb SPSC rings,
  50–170 ns per hop
- **External**: WebSocket JSON for the public API

See [specs/2/1-architecture.md](specs/2/1-architecture.md)
and [specs/2/20-network.md](specs/2/20-network.md).

## Crate layout

12 Rust crates in the cargo workspace; rsx-messages was
split out of rsx-dxs so transport is now domain-agnostic
(zero rsx-types prod dep — only dev-deps).

```
rsx-types/      Price, Qty, Side, SymbolConfig, macros
rsx-dxs/        Domain-agnostic transport: WAL + CMP/UDP +
                DXS/TCP replay; versioned wire header
                (no rsx-types prod dep)
rsx-messages/   Exchange wire records: Fill, BBO, Order*,
                MarkPrice, Liquidation, ConfigApplied
                (22 size+align compile-time asserts)
rsx-book/       Orderbook (Slab, CompressionMap, PriceLevel)
rsx-matching/   ME (per-symbol, single-threaded, core-pinned;
                O(1) cancel via FxHashMap<OrderKey, slab>)
rsx-risk/       Risk (per-shard, full tile arrangement)
rsx-gateway/    Gateway (monoio WS + CMP bridge, hardened JWT
                with min-32B secret + nbf + JtiTracker, bounded
                per-IP rate limit with FIFO eviction)
rsx-marketdata/ Marketdata (monoio shadow book, L2/BBO)
rsx-mark/       Mark price (external feeds, CMP to risk)
rsx-recorder/   Recorder (archival DXS consumer)
rsx-cli/        WAL dump/inspect tool (JSON + parquet)
rsx-maker/      Market-maker bot (two-sided quoting)
```

Non-cargo subprojects:

```
rsx-playground/ Dev dashboard (Python/FastAPI + Playwright)
rsx-webui/      Frontend (Vite + React + Tailwind)
rsx-auth/       Auth service (Python, GitHub OAuth)
```

## Playground

Web dashboard for development. Process control, order
submission, WAL inspection, fault injection, invariant
verification.

```bash
./rsx-playground/playground start     # start FastAPI server
./rsx-playground/playground stop      # stop server
./rsx-playground/playground ps        # list rsx-* processes
./rsx-playground/playground start-all # build + launch
./rsx-playground/playground stop-all  # stop processes
./rsx-playground/playground reset     # stop + clean state
```

Trade UI lives at `/trade/`, all URLs relative — works behind
any reverse-proxy prefix. Detailed docs in
[rsx-playground/README.md](rsx-playground/README.md).

## What's measured vs what's a budget

It's worth being explicit about which numbers in this repo
are measurements and which are design budgets. RSX is not
yet ready to ship "we measured 4 µs end-to-end at line rate"
because the harness that would assert that doesn't exist yet.

| Number                          | Measured?               |
|---------------------------------|-------------------------|
| 54 ns match single fill         | yes — `rsx-book` bench  |
| 31 ns WAL buffer append (no disk I/O; `WalWriter::append` is a `Vec` extend, pre-fsync) | yes — `rsx-dxs` bench   |
| 43 ns protocol-record encode (StatusMessage / Nak / Heartbeat; 23 ns for `FillRecord`), 9 ns decode | yes — `rsx-dxs/cmp_bench` + `rsx-messages/encode_bench` |
| 50–170 ns SPSC ring hop         | yes — `rsx-book` bench  |
| <50 µs GW→ME→GW round trip      | **design budget**; F1 probe + dashboard shipped (commit `bded133`), `make latency-publish` writes p50/p99 to `bench-baseline.json` once cluster-run; WAL-backed NAK retransmit (`366d1b2`) closes the two-tier loss-recovery path |
| <500 ns ME match                | yes — sub-bench of 54 ns |
| <5 µs risk pre-trade            | **design budget**       |

The microbench numbers are single-thread, no-contention
costs. The E2E budget is derived from summing them but
hasn't been gated by a continuous test under load. See
[specs/2/22-perf-verification.md](specs/2/22-perf-verification.md)
for the harness plan and
[`.ship/12-SHOWCASE-HONEST/`](.ship/12-SHOWCASE-HONEST/) for
the in-flight work.

## Build and test

```bash
make check       # cargo check (fastest feedback)
make test        # Rust unit + integration (878 passing, <5s)
make wal         # WAL correctness suite (<10s)
make e2e         # Rust + API + Playwright (~3 min)
make integration # testcontainers (1–5 min)
make lint        # clippy with -D warnings
make perf        # Criterion benches
make bench-gate  # local 10% regression gate
make clean       # cargo clean
```

Single crate: `cargo test -p rsx-book`
Single test: `cargo test -p rsx-book -- test_name`

Test count, current truth: **878 passing** Rust unit +
integration, ~930 Python (rsx-playground), 421 Playwright
(canonical). [PROGRESS.md](PROGRESS.md) is the source of truth.

## Design principles

- **Fixed-point i64** — no float rounding, no NaN
- **One thread per symbol for matching** — no locks
- **SPSC rings between tiles** — rtrb, single producer, single consumer, ~50–170 ns
- **WAL-based recovery** — idempotent replay from tip + 1
- **Slab arena** — pre-allocated, zero heap on hot path
- **WAL = wire = stream** — no format transformation
- **CMP/UDP for inter-process hot path** — direct, no Kafka, no NATS
- **SIGTERM = crash** — one recovery code path

## Specs

All in `specs/2/`. The numbered names are stable; old
unnumbered references (CMP.md, TILES.md, …) were retired.

| Spec                                                            | Covers                                       |
|-----------------------------------------------------------------|----------------------------------------------|
| [specs/2/1-architecture.md](specs/2/1-architecture.md)          | System overview, principles                  |
| [specs/2/4-cmp.md](specs/2/4-cmp.md)                            | C Message Protocol (UDP transport)           |
| [specs/2/6-consistency.md](specs/2/6-consistency.md)            | Event ordering, fan-out, FIFO                |
| [specs/2/10-dxs.md](specs/2/10-dxs.md)                          | DXS replay server (TCP fan-out)              |
| [specs/2/13-liquidator.md](specs/2/13-liquidator.md)            | Liquidation rounds, insurance fund           |
| [specs/2/15-mark.md](specs/2/15-mark.md)                        | Mark price aggregation (external feeds)      |
| [specs/2/16-marketdata.md](specs/2/16-marketdata.md)            | Shadow book, L2/BBO/trades                   |
| [specs/2/18-messages.md](specs/2/18-messages.md)                | Wire record-type catalogue                   |
| [specs/2/20-network.md](specs/2/20-network.md)                  | Process topology, ports                      |
| [specs/2/21-orderbook.md](specs/2/21-orderbook.md)              | Slab + CompressionMap, matching algorithm    |
| [specs/2/22-perf-verification.md](specs/2/22-perf-verification.md) | Bench gate, latency harness plan          |
| [specs/2/28-risk.md](specs/2/28-risk.md)                        | Margin, positions, funding                   |
| [specs/2/45-tiles.md](specs/2/45-tiles.md)                      | Tile architecture, per-process status        |
| [specs/2/48-wal.md](specs/2/48-wal.md)                          | WAL flush rules, rotation, retention         |
| [specs/2/49-webproto.md](specs/2/49-webproto.md)                | Public WebSocket compact JSON protocol       |

[specs/index.md](specs/index.md) is the full table.

## Documentation

| Document                                       | Purpose                              |
|------------------------------------------------|--------------------------------------|
| [PROGRESS.md](PROGRESS.md)                     | Per-crate status, current state      |
| [GUARANTEES.md](GUARANTEES.md)                 | Consistency, durability, ordering    |
| [BLOG.md](BLOG.md)                             | Long-form narrative                  |
| [FEATURES.md](FEATURES.md)                     | Feature matrix                       |
| [TESTING.md](TESTING.md)                       | Test taxonomy and how to run         |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md)       | Failure mode catalogue               |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md)     | Operator recovery procedures         |
| [docs/DEMO.md](docs/DEMO.md)                   | 60-second clean-boot demo            |
| [.ship/12-SHOWCASE-HONEST/](.ship/12-SHOWCASE-HONEST/) | In-flight work to surface novelty |

## What's not done

A short, honest list of the gaps a careful reader will hit:

- **End-to-end latency harness.** The 50 µs / 500 ns numbers
  are budgets, not measurements. Plan in
  [specs/2/22-perf-verification.md](specs/2/22-perf-verification.md).
- **CMP NAK retransmit** is now two-tier (commit `366d1b2`):
  in-memory ring for recent records, WAL-backed for older
  history. The send-ring uses preallocated `Box<[T]>` slabs
  (`7befe76`) — zero heap on the send path. Documented in
  [specs/2/4-cmp.md §10.3](specs/2/4-cmp.md).
- **JWT replay protection**: `JtiTracker` (`a6a92c3`) is
  implemented but not yet wired through `ws_handshake` — the
  type exists, the handshake doesn't consult it. Tracked as
  TODO.
- **`rsx-mark` and `rsx-marketdata` replay** still use tokio.
  Migrating mark to monoio is queued in
  `.ship/12-SHOWCASE-HONEST/`.
- **CMP transport** uses `std::net::UdpSocket`, not monoio
  io_uring. One syscall per `sendto` / `recvfrom` on the
  hot path.
- **rsx-maker** uses blocking `tungstenite`. Fine for a demo;
  not a low-latency client.

## Vibe

<details>
<summary>The Vibe</summary>

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

</details>
