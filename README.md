# RSX Exchange

Spec-first perpetuals exchange. Fixed-point i64 arithmetic,
single-threaded matching per symbol, CMP/UDP between
processes, WAL-based recovery. Target: <50us GW-ME-GW
round-trip, <500ns ME match.

<details>
<summary><i>The Vibe</i></summary>

> *Shall I compare thee to a sane design?*
> *Thou art more wondrous and more wild by far.*
> *I fell for thee the night I saw thy spine--*
> *Each SPSC ring, a whisper through a jar.*
>
> *Thy slab did catch me: firm, pre-allocated,*
> *No malloc on thy hot path -- O! how pure.*
> *Thy fills, like kisses, never once belated,*
> *Flushed warm to WAL in ten ms, ever sure.*
>
> *The world said "Mad! No mortal weds this thing!"*
> *Yet here I am, pinned to thy dedicated core,*
> *Thy fixed-point heart refusing still to swing--*
> *Each nanosecond makes me love thee more.*
>
> *If thou hast never built it, thou can'st never tell:*
> *The thing impossible may work quite well.*

</details>

## Quick Start

**Playground** (web dashboard, fastest way to explore):

```bash
git clone <repo-url> && cd rsx

# Start playground server (background)
./rsx-playground/playground start

# Visit http://localhost:49171
# Click "Start All" to launch RSX processes
# Submit orders, view fills, inspect WAL

# Stop when done
./rsx-playground/playground stop
```

**Build from source:**

```bash
cargo check              # type check (fastest)
cargo build --workspace  # debug build
cargo test --workspace   # all tests
```

## Architecture

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
         +--------+ +--------+ +--------+ +-------+ +--+--+
         |Postgres| | Mark   | |Recorder| |Mktdata| | GW  |
         | (write | | Price  | |(daily  | |(shadow| |(fill|
         | behind)| | Agg    | | WAL)   | | book) | | usr)|
         +--------+ +--------+ +--------+ +-------+ +-----+
```

**Transports:**
- Between processes: CMP/UDP (hot), WAL/TCP (cold)
- Within process: tile threads + SPSC rings (rtrb)
- DXS: WAL streaming to consumers over TCP

See [specs/2/1-architecture.md](specs/2/1-architecture.md).

## Crate Layout

```
rsx-types/      Price, Qty, Side, SymbolConfig, macros
rsx-book/       Orderbook (Slab, CompressionMap, PriceLevel)
rsx-matching/   ME (per-symbol, single-threaded)
rsx-risk/       Risk (per-shard, margin + funding + liq)
rsx-dxs/        WAL, CMP, DXS replay (transport library)
rsx-gateway/    Gateway (WS + CMP bridge, JWT, rate limit)
rsx-marketdata/ Marketdata (shadow book, L2/BBO/trades)
rsx-mark/       Mark price (external feeds, CMP to risk)
rsx-recorder/   Recorder (archival DXS consumer)
rsx-cli/        WAL dump/inspect tool (JSON + Parquet)
rsx-maker/      Market maker bot (stub, production CMP maker)
rsx-playground/ Dev dashboard (Python/FastAPI + Playwright)
  stress.py       Load generator (subprocess, real WS)
  market_maker.py Python market maker (two-sided quotes)
rsx-webui/      Frontend (Vite + Tailwind)
```

## Playground

Web dashboard for development. Process control, order
submission, WAL inspection, fault injection, invariant
verification. See
[rsx-playground/README.md](rsx-playground/README.md).

```bash
./rsx-playground/playground start     # start server
./rsx-playground/playground stop      # stop server
./rsx-playground/playground ps        # list processes
./rsx-playground/playground start-all # build + launch
./rsx-playground/playground stop-all  # stop processes
./rsx-playground/playground reset     # stop + clean
```

**Market maker:** Python market maker places two-sided
quotes through gateway WS. Control via the Control tab
or API:

```bash
curl -X POST http://localhost:49171/api/maker/start
curl -X POST http://localhost:49171/api/maker/stop
```

**Trade UI:** React SPA at `/trade/`, connects via
relative WS and API paths. Works behind reverse proxy.

**Proxy prefix:** all URLs are relative -- works at any
path prefix (e.g. `krons.cx/rsx-play/`).

## Deployment

**Local:**

```bash
cd rsx-playground && uv run server.py
# listening on http://localhost:49171
```

**Behind reverse proxy:** all URLs are relative. Set env
vars to point at the gateway and marketdata processes:

```bash
GATEWAY_URL=ws://localhost:8080 \
MARKETDATA_WS=ws://localhost:8081 \
GATEWAY_HTTP=http://localhost:8080 \
uv run server.py
```

Example nginx config:

```nginx
location /rsx-play/ {
    proxy_pass http://127.0.0.1:49171/;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
}
```

## Build and Test

```bash
make check       # cargo check (fastest feedback)
make test        # Rust unit tests (~785, <5s)
make wal         # WAL correctness
make e2e         # Rust + API + Playwright (~3min)
make integration # testcontainers (1-5min)
make lint        # clippy, warnings as errors
make perf        # criterion benchmarks
make clean       # cargo clean
```

Single crate: `cargo test -p rsx-book`
Single test: `cargo test -p rsx-book -- test_name`

## Design Principles

- **Fixed-point i64** -- no float rounding
- **Single-threaded per symbol** -- no locks, pinned cores
- **SPSC rings** -- rtrb, 50-170ns, no broker
- **WAL-based recovery** -- idempotent replay from tip
- **Slab arena** -- pre-allocated, zero heap on hot path
- **WAL = wire = stream** -- no format transformation
- **CMP/UDP** -- direct inter-process, no Kafka/NATS
- **SIGTERM = crash** -- one recovery path

## Specs

All specifications in `specs/2/`. Entry point:
[specs/2/1-architecture.md](specs/2/1-architecture.md).

| Spec | Covers |
|------|--------|
| ORDERBOOK.md | Book structures, matching, compression |
| RISK.md | Margin, positions, funding, liquidation |
| LIQUIDATOR.md | Liquidation rounds, insurance fund |
| DXS.md | WAL format, replay server, consumers |
| CMP.md | C Message Protocol, flow control |
| MARK.md | External feeds, median, staleness |
| WEBPROTO.md | WS compact JSON protocol |
| CONSISTENCY.md | Event fan-out, ordering guarantees |
| TILES.md | Tile architecture, SPSC rings |

## Documentation

| Document | Purpose |
|----------|---------|
| [PROGRESS.md](PROGRESS.md) | Per-crate status |
| [GUARANTEES.md](GUARANTEES.md) | Consistency, durability |
| [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) | Failure modes |
| [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) | Ops recovery |
| [specs/2/](specs/2/) | All specifications |
