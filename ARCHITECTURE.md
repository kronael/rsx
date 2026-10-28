# RSX System Architecture

RSX is a portfolio-margin derivatives exchange split into independent
processes. Each concern has one owner.

## Process topology

```text
                              public WebSocket
                                     │
                                     ▼
                              ┌─────────────┐
                              │   Gateway   │
                              │ auth, route │
                              └──────┬──────┘
                                     │ casting/UDP
                                     ▼
                       ┌──────────────────────────┐
                       │ Risk shard for user U    │
                       │ margin, positions, funds │
                       └─────────────┬────────────┘
                                     │ casting/UDP
                                     ▼
                       ┌──────────────────────────┐
                       │ ME for symbol S          │
                       │ dedup, WAL, order book   │
                       └─────┬──────────┬─────────┘
                             │          │
                  settle via │          ├──► Marketdata ──► public WebSocket
                       Risk  │          └──► Recorder
                             ▼
                          Gateway ──► client
```

Risk shards by `user_id`. Matching shards by `symbol_id`. Adding users moves
risk virtual shards; adding symbols starts more matching engines. The two
changes do not require each other.

## Process ownership

| Process | Owns | Scale unit |
|---|---|---|
| Gateway | external sessions, JWT validation, request parsing, routing | client connections |
| Risk | portfolio margin, positions, funding, liquidation, persistence | user virtual shards |
| Matching engine | one symbol's order sequence, deduplication, book, WAL, recovery | symbol |
| Marketdata | shadow books and public BBO, depth, and trade fan-out | symbols and subscribers |
| Mark | external price aggregation | feed set |
| Recorder | archival WAL consumption | stream |

The gateway does not own margin or positions. Risk does not match orders.
Matching does not re-run authentication or portfolio checks. Marketdata cannot
backpressure the order round trip.

## Crate ownership

The Cargo workspace contains 14 crates:

| Crate | Responsibility |
|---|---|
| `rsx-types` | fixed-point values, symbol configuration, shared invariants |
| `rsx-cast` | domain-agnostic WAL, casting/UDP, and TCP replay |
| `rsx-messages` | exchange wire records built on `rsx-cast` |
| `rsx-ws` | WebSocket accept loop, upgrade read and frame I/O shared by the gateway and marketdata |
| `rsx-book` | deterministic order-book and matching state |
| `rsx-matching` | matching-engine process logic |
| `rsx-risk` | portfolio-risk process logic |
| `rsx-gateway` | external WebSocket edge |
| `rsx-marketdata` | shadow book and public feed |
| `rsx-mark` | mark-price aggregation |
| `rsx-recorder` | archival replay consumer |
| `rsx-cli` | operator tool: WAL and invariant checks, cluster launcher, orders, stress, faults, config and accounts |
| `rsx-log` | off-path structured-log drain |
| `rsx-health` | health and load snapshots |

`rsx-glass` is the Go desktop trading UI, a GPU renderer over the shared
`rsx-core` model. `rsx-playground` is a Python/FastAPI local
control surface. `rsx-auth` is a separate Python authentication service.

## Order lifecycle

1. The client sends a fixed-point order through the gateway WebSocket.
2. Gateway authenticates, parses, validates the wire shape, assigns the order
   ID, and routes by user.
3. The owning Risk shard checks portfolio margin and reserves capacity.
4. Risk routes the accepted order by symbol to the matching engine.
5. Matching deduplicates the command, applies the deterministic book
   transition, records the result, and publishes ordered events.
6. Risk applies fills and releases reservations.
7. Gateway returns the terminal result to the client.
8. Marketdata and Recorder consume their copies off the critical response
   path.

Fills precede the terminal order event. A single matching engine owns the total
order for each symbol.

## Transport and records

Inter-process traffic has two paths:

- **Live path** — casting/UDP for orders, fills, BBO, and control records.
- **Recovery path** — WAL replication over TCP for replay and archival.

`rsx-cast` uses one fixed record representation for memory, disk, UDP, and TCP
replay. `rsx-messages` defines the exchange-specific payloads. Casting trusts
the internal network; JWT terminates at the external gateway and TLS at the
proxy in front of it, while network policy protects internal peers.

Within a process, pinned threads pass fixed values through SPSC rings. Each
consumer has its own ring. A full required-consumer ring stalls its producer;
events are not silently dropped.

## Matching and order-book boundary

`rsx-book` owns deterministic state:

- price levels and order slots;
- crossing, time-in-force, cancel, and modify transitions;
- exact fixed-point prices and quantities;
- ordered result events;
- logical book checkpoints.

`rsx-matching` owns process concerns:

- transport and command decoding;
- one order sequence per symbol;
- deduplication and order-ID lookup;
- WAL sequence and recovery orchestration;
- checkpoint persistence;
- downstream event encoding and fan-out.

One thread owns each book. No lock is needed inside `rsx-book`.

## Fixed-point boundary

Prices and quantities are `i64` values in raw tick and lot units. The gateway
converts human input at the outer API boundary. Risk checks notional arithmetic
with wider or checked intermediates before matching. The book does not carry
display precision or floating-point values.

## Recovery

- **Matching** restores the latest valid checkpoint, then replays later WAL
  records in sequence.
- **Risk** restores persisted state and replays fills from its durable tip.
- **Marketdata** rebuilds its shadow view from matching events.
- **Gateway** is stateless with respect to exchange balances and positions.
- **Recorder** resumes from its persisted stream position.

The governing durability and loss bounds are in
[GUARANTEES.md](GUARANTEES.md). Failure-specific behavior is in
[CRASH-SCENARIOS.md](CRASH-SCENARIOS.md).

## System invariants

1. Fills precede the terminal order event.
2. Each accepted command has exactly one terminal outcome.
3. Orders execute FIFO within an exact price level.
4. Risk positions equal applied fills.
5. Stream tips never decrease.
6. The book is not crossed after an order transition.
7. SPSC rings preserve producer order.
8. Every allocated slab slot is free or active.
9. Funding is zero-sum per symbol and interval.
10. At most one main process owns a shard lease.

## Operational shape

Configuration comes from environment variables. Hot loops may be pinned to
dedicated cores. Gateway and marketdata use `monoio` for many WebSocket file
descriptors; Risk uses a non-blocking UDP loop; off-path persistence and
control work may use Tokio.

Latency targets and measurement rules live in
[specs/2/22-perf-verification.md](specs/2/22-perf-verification.md). Dated
measurements live in [reports/](reports/); architecture documents do not turn
targets into results.
