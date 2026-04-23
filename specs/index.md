# Specs Index

Master table of all project specs.

## Phase 1 — Shipped ship logs (historical)

Completed ship projects, renamed from specs/done/.

| Spec | Status | Summary |
|------|--------|---------|
| [1/1-coverage.md](1/1-coverage.md) | shipped | updated: Feb 19 2026 |
| [1/2-crash.md](1/2-crash.md) | shipped | Audit date: 2026-02-11 |
| [1/3-critique.md](1/3-critique.md) | shipped | Comprehensive audit of functionality gaps, test quality, |
| [1/4-naming.md](1/4-naming.md) | shipped | Canonical names across DB tables, API endpoints, HTMX partials, |
| [1/5-project.md](1/5-project.md) | shipped | Make the market maker a first-class component in the playground test |
| [1/6-replication-impl.md](1/6-replication-impl.md) | shipped | Implementation of RISK.md §Replication & Failover for rsx-risk engine. |
| [1/7-speedup.md](1/7-speedup.md) | shipped | Bottlenecks in current architecture, ordered by impact. |
| [1/8-cmp-nak.md](1/8-cmp-nak.md) | shipped | `CmpSender::handle_nak` opens a `WalReader` to retransmit dropped |
| [1/9-eighty-percent.md](1/9-eighty-percent.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [1/10-exchange-e2e-fixes.md](1/10-exchange-e2e-fixes.md) | shipped | Five bugs block end-to-end exchange operation. Fix all five. Read |
| [1/11-exchange-e2e.md](1/11-exchange-e2e.md) | shipped | The exchange works end-to-end: a market maker provides resting |
| [1/12-gateway-heartbeat.md](1/12-gateway-heartbeat.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, WebSocket) |
| [1/13-gateway-wiring.md](1/13-gateway-wiring.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [1/14-maker-index-feed.md](1/14-maker-index-feed.md) | shipped | Market maker should quote around the RSX mark/index |
| [1/15-maker-integration.md](1/15-maker-integration.md) | shipped | Make the market maker a first-class part of the playground server and |
| [1/16-marketdata-mark.md](1/16-marketdata-mark.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [1/17-marketdata-ws-broadcast.md](1/17-marketdata-ws-broadcast.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [1/18-me-fanout-marketdata.md](1/18-me-fanout-marketdata.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [1/19-play-latency-tests.md](1/19-play-latency-tests.md) | shipped | Playwright tests that verify the playground server |
| [1/20-play-safety-tests.md](1/20-play-safety-tests.md) | shipped | Comprehensive Playwright tests covering process crash |
| [1/21-playground-audit.md](1/21-playground-audit.md) | shipped | Comprehensive audit of rsx-playground as a minimal viable |
| [1/22-post-only.md](1/22-post-only.md) | shipped | Project: RSX perpetuals exchange (Rust) |
| [1/23-risk-liquidation-wiring.md](1/23-risk-liquidation-wiring.md) | shipped | Project: RSX perpetuals exchange (Rust, CMP/UDP) |
| [1/24-risk-mark-consumer.md](1/24-risk-mark-consumer.md) | shipped | Project: RSX perpetuals exchange (Rust, CMP/UDP) |
| [1/25-rust-maker.md](1/25-rust-maker.md) | shipped | Implement `rsx-maker` as a working market maker that connects to the |
| [1/26-todos-readme.md](1/26-todos-readme.md) | shipped | Bug hunt 2026-02-14: 59 bugs + 33 spec test gaps + 7 future items. |
| [1/27-todos-refinement.md](1/27-todos-refinement.md) | shipped | date: 2026-02-22 (updated) |
| [1/28-trade-ui-fixes.md](1/28-trade-ui-fixes.md) | shipped | The `/trade/` React SPA shows live data for all panels. Read all |
| [1/29-trade-ui.md](1/29-trade-ui.md) | shipped | The `/trade/` SPA works correctly with live RSX processes running. |

## Phase 2 — Current architectural reference

Active specs. Source of truth for system design.

| Spec | Status | Summary |
|------|--------|---------|
| [2/1-architecture.md](2/1-architecture.md) | shipped | Perpetuals exchange. Fixed-point arithmetic, single-threaded |
| [2/2-archive.md](2/2-archive.md) | shipped | Archive serves historical WAL records from flat files on disk. It is used when h |
| [2/3-cli.md](2/3-cli.md) | shipped | `rsx-cli` is an offline WAL debugging tool. It reads WAL files written |
| [2/4-cmp.md](2/4-cmp.md) | shipped | Fixed-size C structs over the network. One wire format for |
| [2/5-codepaths.md](2/5-codepaths.md) | shipped | This document enumerates major end-to-end codepaths and maps them to |
| [2/6-consistency.md](2/6-consistency.md) | shipped | Matching engine produces events into a fixed array buffer. Events fan out |
| [2/7-dashboard.md](2/7-dashboard.md) | shipped | Support-facing dashboard for user-level operations: |
| [2/8-database.md](2/8-database.md) | shipped | - [Recommendation](#recommendation) |
| [2/9-deploy.md](2/9-deploy.md) | shipped | - [Multi-Server Topology](#multi-server-topology) |
| [2/10-dxs.md](2/10-dxs.md) | shipped | Brokerless WAL streaming. Each producer IS the server for its own |
| [2/11-gateway.md](2/11-gateway.md) | shipped | Gateway adapts external clients to internal CMP. It owns |
| [2/12-health-dashboard.md](2/12-health-dashboard.md) | shipped | Systems operations dashboard for platform health: |
| [2/13-liquidator.md](2/13-liquidator.md) | shipped | - [Context](#context) |
| [2/14-management-dashboard.md](2/14-management-dashboard.md) | shipped | This spec is intentionally split into four separate dashboards: |
| [2/15-mark.md](2/15-mark.md) | shipped | Standalone network service. Aggregates mark prices from external |
| [2/16-marketdata.md](2/16-marketdata.md) | shipped | Market data is served by a dedicated service. It consumes orderbook |
| [2/17-matching.md](2/17-matching.md) | shipped | Matching is per-symbol, single-threaded, and stateless with |
| [2/18-messages.md](2/18-messages.md) | shipped | - [Overview](#overview) |
| [2/19-metadata.md](2/19-metadata.md) | shipped | This spec defines how symbol configuration is scheduled and propagated. Matching |
| [2/20-network.md](2/20-network.md) | shipped | - [Overview](#overview) |
| [2/21-orderbook.md](2/21-orderbook.md) | shipped | - [Design Goals](#design-goals-from-todomd) |
| [2/22-perf-verification.md](2/22-perf-verification.md) | shipped | Wire up latency measurement end-to-end in the playground |
| [2/23-playground-dashboard.md](2/23-playground-dashboard.md) | shipped | Developer/testing dashboard for local and staging playground workflows. |
| [2/24-position-edge-cases.md](2/24-position-edge-cases.md) | shipped | Comprehensive catalog of edge cases for position tracking across |
| [2/25-process.md](2/25-process.md) | shipped | - [Scope](#scope) |
| [2/26-rest.md](2/26-rest.md) | shipped | > **Status: v2 — deferred.** Only `/health` and `/v1/symbols` |
| [2/27-risk-dashboard.md](2/27-risk-dashboard.md) | shipped | Risk/operations dashboard for exchange-wide controls: |
| [2/28-risk.md](2/28-risk.md) | shipped | - [Context](#context) |
| [2/29-rpc.md](2/29-rpc.md) | shipped | - [Overview](#overview) |
| [2/30-scenarios.md](2/30-scenarios.md) | shipped | Deployment scenarios for the RSX exchange. Defines which |
| [2/31-sim.md](2/31-sim.md) | shipped | The playground has fake order-matching (`_sim_submit`, |
| [2/33-telemetry.md](2/33-telemetry.md) | shipped | How RSX processes emit metrics and how they reach |
| [2/34-testing-book.md](2/34-testing-book.md) | shipped | Source specs: [ORDERBOOK.md](ORDERBOOK.md), |
| [2/35-testing-cmp.md](2/35-testing-cmp.md) | shipped | Version: 1.0 |
| [2/36-testing-dxs.md](2/36-testing-dxs.md) | shipped | Source specs: [DXS.md](DXS.md), [WAL.md](WAL.md) |
| [2/37-testing-gateway.md](2/37-testing-gateway.md) | shipped | Source specs: [NETWORK.md](NETWORK.md), [WEBPROTO.md](WEBPROTO.md), |
| [2/38-testing-liquidator.md](2/38-testing-liquidator.md) | shipped | Source spec: [LIQUIDATOR.md](LIQUIDATOR.md) |
| [2/39-testing-mark.md](2/39-testing-mark.md) | shipped | Source spec: [MARK.md](MARK.md) |
| [2/40-testing-marketdata.md](2/40-testing-marketdata.md) | shipped | Source specs: [MARKETDATA.md](MARKETDATA.md), |
| [2/41-testing-matching.md](2/41-testing-matching.md) | shipped | Source specs: [ORDERBOOK.md](ORDERBOOK.md), |
| [2/42-testing-risk.md](2/42-testing-risk.md) | shipped | Source spec: [RISK.md](RISK.md) |
| [2/43-testing-smrb.md](2/43-testing-smrb.md) | shipped | Source specs: [notes/SMRB.md](../../notes/SMRB.md), |
| [2/44-testing.md](2/44-testing.md) | shipped | For comprehensive edge case documentation across all validation layers, |
| [2/45-tiles.md](2/45-tiles.md) | shipped | - [Overview](#overview) |
| [2/46-trade-ui.md](2/46-trade-ui.md) | shipped | Trade UI integration issues and fix plan. |
| [2/47-validation-edge-cases.md](2/47-validation-edge-cases.md) | shipped | Comprehensive documentation of edge cases for order validation |
| [2/48-wal.md](2/48-wal.md) | shipped | > **Note:** The concrete WAL implementation (file format, writer, |
| [2/49-webproto.md](2/49-webproto.md) | shipped | Gateway exposes a compact WebSocket protocol and translates |

## Phase 3 — Future / archival

Future plans, archival notes, observed implementation.

| Spec | Status | Summary |
|------|--------|---------|
| [3/1-future.md](3/1-future.md) | reference | This document collects optimization ideas and protocol improvements that are |
| [3/2-implementation.md](3/2-implementation.md) | reference | This document captures notable implementation details that are now |
| [3/3-orderbookv2.md](3/3-orderbookv2.md) | reference | **Status:** Not planned. This document is archival only. |

