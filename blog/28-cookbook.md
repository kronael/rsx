# RSX Operations Cookbook: Recipes for Driving the Exchange

A task-oriented companion to the design posts. (For how the exchange was
*built* — spec-first, agent-driven — see
[Building RSX](29-building-rsx.md), the vibe-book.) Each recipe is a
self-contained "I want to X → do this" with the exact commands. Every
command here is what the dev path actually runs — no pseudo-code.

The assumptions throughout: debug binaries built
(`cargo build --workspace`), Postgres reachable at
`postgres://rsx:rsx@127.0.0.1:5432/rsx`, and the playground venv ready
(`make prepare`, one-time). The playground dashboard is the single front
door at `http://localhost:49171`.

---

## 1. Boot the whole exchange from a clean slate

```bash
# stop anything stale (idempotent)
./rsx-playground/playground stop-all
./rsx-playground/playground reset      # stop + wipe state

# bring up the dashboard, then every RSX process
./rsx-playground/playground start
curl -X POST 'http://localhost:49171/api/processes/all/start?scenario=minimal' \
     -H 'x-confirm: yes'
```

`scenario=minimal` boots the smallest useful topology: one gateway, one
risk shard, one matching engine (symbol PENGU, id 10), marketdata, mark,
recorder. Within a couple of seconds `/api/processes` reports every
process `Running`.

```bash
curl -s http://localhost:49171/api/processes \
  | python3 -c 'import json,sys; [print(p["name"], p["state"]) for p in json.load(sys.stdin)]'
```

---

## 2. Make the book two-sided

An empty book is boring. Start the built-in maker; it quotes around
mid = 50000 on PENGU and refreshes ~20 orders/sec.

```bash
curl -X POST 'http://localhost:49171/api/maker/start' -H 'x-confirm: yes'

# confirm it's quoting (orders_placed climbs)
curl -s http://localhost:49171/api/maker/status
```

If the maker stalls on UDP send errors, the host socket buffers are too
small for the quote rate — `make tune-host` raises `rmem_max` to 25 MB
(needs sudo, one-time per boot).

---

## 3. Submit an order by hand

The gateway speaks WebSocket. The quickest hand-test is the playground's
order form (`http://localhost:49171/`, Order panel), which mints a dev
JWT and submits over `/ws/private`. Programmatically, point any WS client
at the gateway with a JWT signed by `RSX_GW_JWT_SECRET`:

```jsonc
// → ws://<gateway>/ws/private  (after JWT handshake)
{ "type": "order",
  "cid": "my-client-order-001",   // 20-char client id, your idempotency key
  "symbol": 10,                    // symbol_id (u32), not a ticker string
  "side": "buy",
  "px": 50000,                     // fixed-point i64, in tick units
  "qty": 1,                        // fixed-point i64, in lot units
  "tif": "gtc" }                   // gtc | ioc | fok ; post_only / reduce_only flags
```

You get back `ORDER_ACCEPTED` (with the exchange `oid`, a UUIDv7), then
`FILL`s (each precedes completion), then exactly one of `ORDER_DONE` /
`ORDER_FAILED`. Prices and quantities are **fixed-point i64 in
smallest units** — convert at the API boundary only
(`px_raw = human_px / tick_size`). Never send a float.

---

## 4. Watch the tape and the depth

```bash
open http://localhost:49171/         # Process Control + live book
open http://localhost:49171/trade/   # the React Trade UI (SPA)
```

The Trade UI connection dot turns green once `/ws/private` is up; the
book at PENGU fills with the maker's quotes within ~30s.

---

## 5. Read the WAL

The WAL on disk **is** the wire format **is** the replay stream — same
bytes, no transformation. Dump it with the CLI:

```bash
# human-readable record dump for symbol PENGU (id 10)
cargo run -q -p rsx-cli -- dump tmp/wal/pengu/10/10_active.wal | head -40

# just count records (grows as orders flow)
cargo run -q -p rsx-cli -- dump tmp/wal/pengu/10/10_active.wal | wc -l
```

Records are a 16-byte header (`version` at offset 0, `record_type`,
`len`, CRC32C) + a `#[repr(C, align(64))]` payload. See
[We Deleted the Serialization Layer](12-deleted-serialization.md) for why
there's no decoder.

---

## 6. Check health, readiness, and saturation

Every daemon exposes a tiny std-only HTTP health server when you set its
`*_HEALTH_ADDR` env var. The dev launcher wires these by default:

| Process    | Health addr env             | Default port |
|------------|-----------------------------|--------------|
| gateway    | `RSX_GW_HEALTH_ADDR`        | `9200`       |
| risk       | `RSX_RISK_HEALTH_ADDR`      | `9201`       |
| matching   | `RSX_ME_HEALTH_ADDR`        | `9202`       |
| marketdata | `RSX_MARKETDATA_HEALTH_ADDR`| `9203`       |
| mark       | `RSX_MARK_HEALTH_ADDR`      | `9204`       |
| recorder   | `RSX_RECORDER_HEALTH_ADDR`  | `9205`       |

```bash
curl -s 127.0.0.1:9201/health    # 200 = live, 503 = restart this pod
curl -s 127.0.0.1:9201/ready     # 503 = overloaded → k8s sheds it from the Service
curl -s 127.0.0.1:9201/metrics   # JSON: queue fullness + saturation gauges (HPA input)
```

`/ready` returning 503 is the **scale-out signal**: queues are filling, so
k8s removes the pod from the load-balancer (sheds new connections) without
killing it. `/metrics` is the HPA's numeric input — saturation and ring
occupancy as relaxed-atomic gauges, dumped as JSON (no Prometheus).

---

## 7. Inject a fault and watch recovery

The playground can inject faults to exercise the crash/recovery paths
documented in [WAL and Recovery](04-wal-and-recovery.md) and
`CRASH-SCENARIOS.md`:

```bash
# kill a single process by name; the supervisor + WAL replay bring it back
curl -X POST 'http://localhost:49171/api/processes/risk/stop' -H 'x-confirm: yes'
curl -X POST 'http://localhost:49171/api/processes/risk/start' -H 'x-confirm: yes'
```

On restart, risk re-loads positions from Postgres, then warm-catches-up
from the matching engine's WAL replication stream, and only takes the
per-shard advisory lock (`pg_try_advisory_lock`, invariant #10: one main
per shard) once it is caught up. SIGTERM is treated as a crash — there is
exactly one recovery path, never a separate "clean shutdown" branch.

---

## 8. Run the test tiers

```bash
make test          # Rust unit tests (--lib only), <5s — every commit
make wal           # WAL correctness (rsx-cast), <10s
make e2e           # Rust + Python API + Playwright, ~3min — every PR
make integration   # testcontainers (real Postgres), 1-5min
make smoke         # against a deployed system, <1min
make lint          # clippy -D warnings
make perf          # Criterion benchmarks (nightly)
```

The acceptance pipeline is four gates run in order — `make gate` runs all
four; never run `gate-4` directly. `make status-doctor` is required before
touching `PROGRESS.md`.

---

## 9. Shut it down

```bash
curl -X POST http://localhost:49171/api/processes/all/stop -H 'x-confirm: yes'
./rsx-playground/playground stop-all
./rsx-playground/playground reset    # also wipes WAL + PG state
```

---

## Gotchas worth internalizing

- **Symbols are `u32` ids, not strings.** PENGU is `10`. The wire never
  carries a ticker.
- **Everything is fixed-point i64.** Convert at the API boundary; the hot
  path never sees a float.
- **`cid` is your idempotency key.** Reusing a `cid` is how the exchange
  dedupes a retried submit (dedup is persisted in the WAL on accept).
- **The dev JWT secret is not safe for anything internet-facing.**
  `RSX_GW_JWT_SECRET=rsx-dev-secret-not-for-prod` and
  `PLAYGROUND_ALLOW_INSECURE_USER_ID=1` are dev-only; clear both for a
  real deploy.
- **Backpressure stalls, it never drops.** If a ring fills, the producer
  waits. A visible stall beats invisible data loss — see
  [Backpressure or Death](15-backpressure-or-death.md).

---

For the 60-second guided version of recipes 1–4, see
[`docs/demo.md`](../docs/demo.md). For *why* any of this is shaped the way
it is, start at [Design Philosophy](01-design-philosophy.md).
