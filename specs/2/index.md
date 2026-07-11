# Specs — Phase 2

Master index for the 53 phase-2 specs. Phase 2 is the
current architecture. See [../index.md](../index.md) for
phase-1 historical specs.

## Status legend

- **shipped** — implemented and in production code today
- **partial** — partially implemented; the spec is real, the
  code only covers part of it
- **draft** — written but not implemented
- **reference** — non-implementation reference material
  (edge-case catalogs, decisions, comparisons)

## Foundations

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 1 | [1-architecture.md](1-architecture.md) | shipped | Derivatives exchange: fixed-point i64, single-threaded ME, casting/UDP, WAL recovery |
| 25 | [25-process.md](25-process.md) | shipped | Process + tile composition per binary |
| 45 | [45-tiles.md](45-tiles.md) | partial | Pinned-thread tile architecture |
| 20 | [20-network.md](20-network.md) | shipped | System network topology |

## Wire + transport (rsx-cast)

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 4 | [4-cast.md](4-cast.md) | shipped | C Message Protocol over UDP — WAL records on the wire |
| 10 | [10-replication.md](10-replication.md) | shipped | replication — brokerless WAL streaming over TCP |
| 18 | [18-messages.md](18-messages.md) | shipped | Wire record definitions (FillRecord, BBO, OrderUpdate, ...) |
| 48 | [48-wal.md](48-wal.md) | shipped | WAL infrastructure — header format, fsync cadence, rotation |
| 6 | [6-consistency.md](6-consistency.md) | shipped | Event fan-out consistency (matching engine → consumers) |
| 49 | [49-webproto.md](49-webproto.md) | shipped | WebSocket overlay wire protocol |
| 29 | [29-rpc.md](29-rpc.md) | shipped | Async RPC request handling |
| 51 | [51-cmp-v2-multicast.md](51-cmp-v2-multicast.md) | draft | casting v2 — multicast streaming |
| 53 | [53-read-service.md](53-read-service.md) | draft | WAL/Archive read service |
| 58 | [58-http3.md](58-http3.md) | spec | HTTP/3 (QUIC) transport binding for the 49-webproto protocol |

## Exchange components

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 11 | [11-gateway.md](11-gateway.md) | shipped | Gateway service — WS ingress + casting bridge + JWT |
| 17 | [17-matching.md](17-matching.md) | shipped | Matching engine — per-symbol, single-threaded |
| 21 | [21-orderbook.md](21-orderbook.md) | shipped | Orderbook data structures + matching algorithm |
| 28 | [28-risk.md](28-risk.md) | partial | Risk engine — portfolio margin per user shard (return path being moved to ME→GW-direct) |
| 16 | [16-marketdata.md](16-marketdata.md) | shipped | Market data service — shadow book, L2/BBO/trades |
| 15 | [15-mark.md](15-mark.md) | shipped | Mark price aggregator (Binance + Coinbase + ...) |
| 13 | [13-liquidator.md](13-liquidator.md) | partial | Liquidator |
| 19 | [19-metadata.md](19-metadata.md) | shipped | Symbol config scheduling + propagation |
| 57 | [57-config-server.md](57-config-server.md) | spec | Dedicated config server replaces ME's direct-Postgres config |
| 24 | [24-position-edge-cases.md](24-position-edge-cases.md) | reference | Position tracking edge cases |
| 47 | [47-validation-edge-cases.md](47-validation-edge-cases.md) | shipped | Order validation edge cases |

## CLI + tooling

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 3 | [3-cli.md](3-cli.md) | shipped | rsx-cli — offline WAL inspection tool |
| 5 | [5-codepaths.md](5-codepaths.md) | shipped | End-to-end codepath catalog |

## Dashboards (web UIs)

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 7 | [7-dashboard.md](7-dashboard.md) | draft | User management (support) |
| 12 | [12-health-dashboard.md](12-health-dashboard.md) | draft | Health (systems ops) |
| 23 | [23-playground-dashboard.md](23-playground-dashboard.md) | draft | Playground (dev/test control plane) |
| 27 | [27-risk-dashboard.md](27-risk-dashboard.md) | draft | Risk ops |
| 54 | [54-tui-access.md](54-tui-access.md) | partial | Terminal access — Playground `/terminal` is implemented; production SSH/web deployment remains separate |
| 55 | [55-terminal.md](55-terminal.md) | partial | Trade terminal UX — perps screen, new-trader bar, multi-market vision (acct/options/sfdx/lend) |
| 56 | [56-network-edge-scaling.md](56-network-edge-scaling.md) | spec | Network-edge I/O scaling — SQPOLL gated on core config + userspace-UDP (cast decoupling) |
| 60 | [60-terminal-assistant.md](60-terminal-assistant.md) | draft | Terminal assistant — Claude Code agent in rsx-term via arizuko's unchanged runner (vantage blocks + MCP fetch/control + agent folder) |

## REST + deploy

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 26 | [26-rest.md](26-rest.md) | partial | REST API (deferred — only `/health` + `/v1/symbols`) |
| 9 | [9-deploy.md](9-deploy.md) | partial | Deployment specification |
| 30 | [30-scenarios.md](30-scenarios.md) | shipped | Deployment scenarios |
| 8 | [8-database.md](8-database.md) | reference | Database choice for positions |

## Performance + telemetry

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 22 | [22-perf-verification.md](22-perf-verification.md) | partial | How latency is measured, gated, surfaced |
| 33 | [33-telemetry.md](33-telemetry.md) | partial | Metrics emission + shipping |
| 59 | [59-latency-observability.md](59-latency-observability.md) | planned | Per-hop record timestamps (per-event latency) + Prometheus aggregate (via 33) |

## Test specs

One per implementation spec, cross-referenced.

| # | Spec | Status | Source |
|---|------|--------|--------|
| 34 | [34-testing-book.md](34-testing-book.md) | shipped | [21-orderbook.md](21-orderbook.md) |
| 35 | [35-testing-cast.md](35-testing-cast.md) | shipped | [4-cast.md](4-cast.md) |
| 36 | [36-testing-replication.md](36-testing-replication.md) | shipped | [10-replication.md](10-replication.md), [48-wal.md](48-wal.md) |
| 37 | [37-testing-gateway.md](37-testing-gateway.md) | shipped | [20-network.md](20-network.md), [49-webproto.md](49-webproto.md), [29-rpc.md](29-rpc.md), [18-messages.md](18-messages.md) |
| 38 | [38-testing-liquidator.md](38-testing-liquidator.md) | shipped | [13-liquidator.md](13-liquidator.md) |
| 39 | [39-testing-mark.md](39-testing-mark.md) | shipped | [15-mark.md](15-mark.md) |
| 40 | [40-testing-marketdata.md](40-testing-marketdata.md) | shipped | [16-marketdata.md](16-marketdata.md) |
| 41 | [41-testing-matching.md](41-testing-matching.md) | shipped | [21-orderbook.md](21-orderbook.md), [6-consistency.md](6-consistency.md) |
| 42 | [42-testing-risk.md](42-testing-risk.md) | shipped | [28-risk.md](28-risk.md) |
| 43 | [43-testing-smrb.md](43-testing-smrb.md) | reference | [../../notes/SMRB.md](../../notes/SMRB.md) |
| 44 | [44-testing.md](44-testing.md) | shipped | Testing strategy + edge-case catalog overview |

## Numbering

Specs are numbered by historical addition order, not topic.
The gaps (2, 31, 32) are intentional — files were renamed or
moved. See `git log specs/2/` if you need a specific
ancestry.
