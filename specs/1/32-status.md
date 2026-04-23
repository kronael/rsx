---
status: shipped
---

# STATUS.md — Component Audit (Feb 27 2026)

Spec-vs-implementation audit across all RSX components.

---

## Summary

785 Rust tests, 1034 Python, 398 Playwright. All crates
compile. Core exchange path (GW→Risk→ME→MD) works for
single-symbol scenarios. Multi-symbol routing is broken.

---

## Component Status

### rsx-types — Complete

Specs: ORDERBOOK.md §1, TILES.md
Price, Qty, SymbolConfig, Side, newtypes, panic handler.
No gaps.

### rsx-book — Complete

Specs: ORDERBOOK.md
Slab, compression map, price levels, matching (IOC/FOK/RO),
snapshot save/load, user position tracking. 16 test files.
No gaps.

### rsx-matching — Complete

Specs: MATCHING.md, ORDERBOOK.md §2.8, DXS.md
WAL integration, dedup tracker, config polling, CMP wiring.
No gaps.

### rsx-dxs — 99%

Specs: DXS.md, WAL.md, CMP.md
WalWriter/Reader, DxsReplay TCP, DxsConsumer, CMP/UDP,
all 14 record types, 16 WAL edge cases handled.

Gap: archive fallback (DXS.md §10.5) not implemented.
Documented in spec as known.

### rsx-gateway — 97%

Specs: GATEWAY.md, WEBPROTO.md, RPC.md, MESSAGES.md
WS ingress, JWT auth, rate limiting, circuit breaker,
order pending map, heartbeat, pre-validation.

Gap: REST API — only `/health` and `/v1/symbols`.
REST.md now labeled "v2 — deferred". GATEWAY.md §Post-MVP
explicitly defers these.

### rsx-risk — 99%

Specs: RISK.md, LIQUIDATOR.md
Position/margin/funding/liquidation, Postgres persistence,
advisory lease failover, replica promotion, WAL replay.

Gap: multi-symbol ME routing. `RSX_ME_CMP_ADDR` is single
address. Needs `RSX_ME_CMP_ADDRS` comma-separated with
symbol_id routing. Documented in SCENARIOS.md.

Gap: 32 `#[ignore]` Postgres tests. Need testcontainers
wiring in `make integration`.

### rsx-marketdata — 98%

Specs: MARKETDATA.md
Shadow book, WS subscriptions, seq gap detection, BBO/L2/
trades channels.

Gap: same multi-symbol routing as risk. Single ME address
hardcoded.

### rsx-mark — Complete

Specs: MARK.md
Aggregation loop, BinanceSource, staleness sweep, WAL +
CMP output.

Gap: combined Binance stream URL for multi-symbol. Minor.

### rsx-recorder — Complete

Specs: DXS.md §8
Daily rotation, DxsConsumer-based archival.

Gap: not in `build_spawn_plan()` — never auto-started.
Documented in SCENARIOS.md.

### rsx-cli — Partial

Specs: CLI.md
WAL dump with all 14 record types decoded (including
LIQUIDATION). Text and JSON output.

Gap: CLI.md proposed improvements (--type, --symbol,
--user, --stats, --follow, --tick-size) not implemented.
Acceptance criteria in CLI.md claim these exist — should
be labeled "proposed" in criteria too.

### rsx-maker — Complete, no spec

No dedicated spec. Working binary: two-sided quoting,
WS client, reconnect, SIGTERM shutdown. Managed by
playground as subprocess.

Recommendation: write MAKER.md if behavior matters for
testing scenarios.

### rsx-playground — Updated

Specs: PLAYGROUND-DASHBOARD.md, SIM.md
Sim mode removed. Stress generator is managed subprocess.
1034 Python tests pass, 398 Playwright tests.

Gap: PLAYGROUND-DASHBOARD.md uses `/v1/api/play/*` paths
but server uses `/api/*`. Spec is aspirational, not
current.

Gap: spec lists fault injection, CMP flows, invariants
screens as "required (initial)" — not implemented.

### stress.py — Complete

Specs: SIM.md §3
Standalone subprocess, env var config, periodic stats,
JSON reports, exit code on p99 target. Matches spec.

---

## Cross-Cutting Issues

### 1. Multi-symbol routing (blocking for scenarios)

All components hardcode single ME address. Affects:
- rsx-risk: `RSX_ME_CMP_ADDR` → needs `RSX_ME_CMP_ADDRS`
- rsx-marketdata: same
- rsx-mark: Binance URL uses `symbols[0]` only
- start script: `build_spawn_plan()` wires single addr

This blocks duo/full/stress scenarios. SCENARIOS.md
documents all 6 implementation tasks needed.

### 2. Recorder not auto-started

Binary works but no scenario spawns it. Add to
`build_spawn_plan()`.

### 3. Integration test infrastructure

32 rsx-risk `#[ignore]` tests need Postgres. `make
integration` doesn't pass `--ignored`. Need
testcontainers wiring.

### 4. Stale spec content (fixed this audit)

- TESTING.md: 843 → 877 tests (fixed)
- REST.md: added "v2 — deferred" label (fixed)

### 5. Missing specs

- MAKER.md — no spec for rsx-maker behavior
- CLI.md acceptance criteria overstate implementation

---

## Priority Order

1. Multi-symbol routing (unblocks scenarios)
2. Scenarios implementation (SCENARIOS.md tasks)
3. Trade UI fixes (TRADE-UI.md)
4. CLI improvements (CLI.md proposed)
5. Integration test wiring
6. REST API (v2, deferred)
