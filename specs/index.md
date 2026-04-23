# Specs Index

Master table of all project specs.

| Spec | Status | Summary |
|------|--------|---------|
| [1/1-architecture.md](1/1-architecture.md) | shipped | Perpetuals exchange. Fixed-point arithmetic, single-threaded |
| [1/2-archive.md](1/2-archive.md) | shipped | Archive serves historical WAL records from flat files on disk. It is used when h |
| [1/3-cli.md](1/3-cli.md) | shipped | `rsx-cli` is an offline WAL debugging tool. It reads WAL files written |
| [1/4-cmp.md](1/4-cmp.md) | shipped | Fixed-size C structs over the network. One wire format for |
| [1/5-codepaths.md](1/5-codepaths.md) | shipped | This document enumerates major end-to-end codepaths and maps them to |
| [1/6-consistency.md](1/6-consistency.md) | shipped | Matching engine produces events into a fixed array buffer. Events fan out |
| [1/7-dashboard.md](1/7-dashboard.md) | shipped | Support-facing dashboard for user-level operations: |
| [1/8-database.md](1/8-database.md) | shipped | - [Recommendation](#recommendation) |
| [1/9-deploy.md](1/9-deploy.md) | shipped | - [Multi-Server Topology](#multi-server-topology) |
| [1/10-dxs.md](1/10-dxs.md) | shipped | Brokerless WAL streaming. Each producer IS the server for its own |
| [1/11-gateway.md](1/11-gateway.md) | shipped | Gateway adapts external clients to internal CMP. It owns |
| [1/12-health-dashboard.md](1/12-health-dashboard.md) | shipped | Systems operations dashboard for platform health: |
| [1/13-liquidator.md](1/13-liquidator.md) | shipped | - [Context](#context) |
| [1/14-management-dashboard.md](1/14-management-dashboard.md) | shipped | This spec is intentionally split into four separate dashboards: |
| [1/15-mark.md](1/15-mark.md) | shipped | Standalone network service. Aggregates mark prices from external |
| [1/16-marketdata.md](1/16-marketdata.md) | shipped | Market data is served by a dedicated service. It consumes orderbook |
| [1/17-matching.md](1/17-matching.md) | shipped | Matching is per-symbol, single-threaded, and stateless with |
| [1/18-messages.md](1/18-messages.md) | shipped | - [Overview](#overview) |
| [1/19-metadata.md](1/19-metadata.md) | shipped | This spec defines how symbol configuration is scheduled and propagated. Matching |
| [1/20-network.md](1/20-network.md) | shipped | - [Overview](#overview) |
| [1/21-orderbook.md](1/21-orderbook.md) | shipped | - [Design Goals](#design-goals-from-todomd) |
| [1/22-perf-verification.md](1/22-perf-verification.md) | shipped | Wire up latency measurement end-to-end in the playground |
| [1/23-playground-dashboard.md](1/23-playground-dashboard.md) | shipped | Developer/testing dashboard for local and staging playground workflows. |
| [1/24-position-edge-cases.md](1/24-position-edge-cases.md) | shipped | Comprehensive catalog of edge cases for position tracking across |
| [1/25-process.md](1/25-process.md) | shipped | - [Scope](#scope) |
| [1/26-rest.md](1/26-rest.md) | shipped | > **Status: v2 — deferred.** Only `/health` and `/v1/symbols` |
| [1/27-risk-dashboard.md](1/27-risk-dashboard.md) | shipped | Risk/operations dashboard for exchange-wide controls: |
| [1/28-risk.md](1/28-risk.md) | shipped | - [Context](#context) |
| [1/29-rpc.md](1/29-rpc.md) | shipped | - [Overview](#overview) |
| [1/30-scenarios.md](1/30-scenarios.md) | shipped | Deployment scenarios for the RSX exchange. Defines which |
| [1/31-sim.md](1/31-sim.md) | shipped | The playground has fake order-matching (`_sim_submit`, |
| [1/32-status.md](1/32-status.md) | shipped | Spec-vs-implementation audit across all RSX components. |
| [1/33-telemetry.md](1/33-telemetry.md) | shipped | How RSX processes emit metrics and how they reach |
| [1/34-testing-book.md](1/34-testing-book.md) | shipped | Source specs: [ORDERBOOK.md](ORDERBOOK.md), |
| [1/35-testing-cmp.md](1/35-testing-cmp.md) | shipped | Version: 1.0 |
| [1/36-testing-dxs.md](1/36-testing-dxs.md) | shipped | Source specs: [DXS.md](DXS.md), [WAL.md](WAL.md) |
| [1/37-testing-gateway.md](1/37-testing-gateway.md) | shipped | Source specs: [NETWORK.md](NETWORK.md), [WEBPROTO.md](WEBPROTO.md), |
| [1/38-testing-liquidator.md](1/38-testing-liquidator.md) | shipped | Source spec: [LIQUIDATOR.md](LIQUIDATOR.md) |
| [1/39-testing-mark.md](1/39-testing-mark.md) | shipped | Source spec: [MARK.md](MARK.md) |
| [1/40-testing-marketdata.md](1/40-testing-marketdata.md) | shipped | Source specs: [MARKETDATA.md](MARKETDATA.md), |
| [1/41-testing-matching.md](1/41-testing-matching.md) | shipped | Source specs: [ORDERBOOK.md](ORDERBOOK.md), |
| [1/42-testing-risk.md](1/42-testing-risk.md) | shipped | Source spec: [RISK.md](RISK.md) |
| [1/43-testing-smrb.md](1/43-testing-smrb.md) | shipped | Source specs: [notes/SMRB.md](../../notes/SMRB.md), |
| [1/44-testing.md](1/44-testing.md) | shipped | For comprehensive edge case documentation across all validation layers, |
| [1/45-tiles.md](1/45-tiles.md) | shipped | - [Overview](#overview) |
| [1/46-trade-ui.md](1/46-trade-ui.md) | shipped | Trade UI integration issues and fix plan. |
| [1/47-validation-edge-cases.md](1/47-validation-edge-cases.md) | shipped | Comprehensive documentation of edge cases for order validation |
| [1/48-wal.md](1/48-wal.md) | shipped | > **Note:** The concrete WAL implementation (file format, writer, |
| [1/49-webproto.md](1/49-webproto.md) | shipped | Gateway exposes a compact WebSocket protocol and translates |
| [2/1-future.md](2/1-future.md) | reference | This document collects optimization ideas and protocol improvements that are |
| [2/2-implementation.md](2/2-implementation.md) | reference | This document captures notable implementation details that are now |
| [2/3-orderbookv2.md](2/3-orderbookv2.md) | reference | **Status:** Not planned. This document is archival only. |

## Archived ship logs

| Spec | Status | Summary |
|------|--------|---------|
| [done/COVERAGE.md](done/COVERAGE.md) | shipped | updated: Feb 19 2026 |
| [done/CRASH.md](done/CRASH.md) | shipped | Audit date: 2026-02-11 |
| [done/CRITIQUE.md](done/CRITIQUE.md) | shipped | Comprehensive audit of functionality gaps, test quality, |
| [done/NAMING.md](done/NAMING.md) | shipped | Canonical names across DB tables, API endpoints, HTMX partials, |
| [done/PROJECT.md](done/PROJECT.md) | shipped | Make the market maker a first-class component in the playground test |
| [done/REPLICATION-IMPL.md](done/REPLICATION-IMPL.md) | shipped | Implementation of RISK.md §Replication & Failover for rsx-risk engine. |
| [done/SPEEDUP.md](done/SPEEDUP.md) | shipped | Bottlenecks in current architecture, ordered by impact. |
| [done/cmp-nak.md](done/cmp-nak.md) | shipped | `CmpSender::handle_nak` opens a `WalReader` to retransmit dropped |
| [done/eighty-percent.md](done/eighty-percent.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [done/exchange-e2e-fixes.md](done/exchange-e2e-fixes.md) | shipped | Five bugs block end-to-end exchange operation. Fix all five. Read |
| [done/exchange-e2e.md](done/exchange-e2e.md) | shipped | The exchange works end-to-end: a market maker provides resting |
| [done/gateway-heartbeat.md](done/gateway-heartbeat.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, WebSocket) |
| [done/gateway-wiring.md](done/gateway-wiring.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [done/maker-index-feed.md](done/maker-index-feed.md) | shipped | Market maker should quote around the RSX mark/index |
| [done/maker-integration.md](done/maker-integration.md) | shipped | Make the market maker a first-class part of the playground server and |
| [done/marketdata-mark.md](done/marketdata-mark.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [done/marketdata-ws-broadcast.md](done/marketdata-ws-broadcast.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [done/me-fanout-marketdata.md](done/me-fanout-marketdata.md) | shipped | Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP) |
| [done/play-latency-tests.md](done/play-latency-tests.md) | shipped | Playwright tests that verify the playground server |
| [done/play-safety-tests.md](done/play-safety-tests.md) | shipped | Comprehensive Playwright tests covering process crash |
| [done/playground-audit.md](done/playground-audit.md) | shipped | Comprehensive audit of rsx-playground as a minimal viable |
| [done/post-only.md](done/post-only.md) | shipped | Project: RSX perpetuals exchange (Rust) |
| [done/risk-liquidation-wiring.md](done/risk-liquidation-wiring.md) | shipped | Project: RSX perpetuals exchange (Rust, CMP/UDP) |
| [done/risk-mark-consumer.md](done/risk-mark-consumer.md) | shipped | Project: RSX perpetuals exchange (Rust, CMP/UDP) |
| [done/rust-maker.md](done/rust-maker.md) | shipped | Implement `rsx-maker` as a working market maker that connects to the |
| [done/todos-README.md](done/todos-README.md) | shipped | Bug hunt 2026-02-14: 59 bugs + 33 spec test gaps + 7 future items. |
| [done/todos-REFINEMENT.md](done/todos-REFINEMENT.md) | shipped | date: 2026-02-22 (updated) |
| [done/trade-ui-fixes.md](done/trade-ui-fixes.md) | shipped | The `/trade/` React SPA shows live data for all panels. Read all |
| [done/trade-ui.md](done/trade-ui.md) | shipped | The `/trade/` SPA works correctly with live RSX processes running. |
