# Playground adversarial audit — 32 findings (2026-07-05)

Read-only browser walkthrough of every page + flow. 10 BROKEN, 17 CONFUSING,
5 POLISH. Screenshots in scratchpad/shots/. Split below into **DASH** (fixable
in rsx-playground pages.py/server.py) and **SYS** (deep system bug — record +
fix carefully, some hot-path / Phase-2).

## BROKEN
1. **[SYS] Recorder "healthy" while dead.** Process table/topology/component/feed
   all say running+healthy, but recorder log ends 21:26 `BLOCKED: 21 consecutive
   stream errors exhausted retry budget (20): No such file` — can't fetch rotated
   WAL from ME repl (9710 can't serve old seq). Health must reflect replication
   liveness (last-consumed-seq advancing), and recorder must catch up from cold
   WAL random-access when it starts behind retention.
2. **[DASH] Risk Lookup ignores user_id.** `/x/risk-user?user_id=1|5|99|7777`
   all return the identical row. Filter the query by user_id.
3. **[SYS] Verify FAIL: WAL self-consistency (shadow vs WAL BBO) 1/1 mismatch.**
   marketdata shadow book diverges from WAL BBO — tied to continuous mktdata
   `WRN seq gap sym=10` (dropped casts). rcvbuf/keep-up; shadow book missing events.
4. **[DASH] Custom Order can't place resting GTC.** Any GTC → `rejected reason=4`;
   risk log: `order notional overflow price=49500000000 qty=1000000000` — the form
   re-scales price/qty so price*qty overflows i64. Fix the form's unit scaling +
   overflow-check at entry with a real message.
5. **[DASH] Custom Order defaults invalid.** Prefilled `QTY 1.0` → `qty not aligned
   to lot (100000)`. Prefill a valid lot-aligned qty + label the unit.
6. **[DASH] Order Lifecycle Trace stuck "pending".** Traced a FILLED order's cid →
   shows submitted→pending "awaiting gateway" forever, same for rejected. Wire to
   real ORDER_ACCEPTED/FILL/ORDER_DONE/ORDER_FAILED.
7. **[DASH] WAL Dump JSON OOMs + shows nothing.** Reads entire ~10GB archive into
   RAM → `out of memory`; `hx-swap="none"` so no output. Stream/paginate + render.
8. **[DASH] WAL Verify button: zero feedback.** `hx-swap="none"`; API returns
   `verified 3 streams` but nothing renders. Target a result element.
9. **[DASH] Stress test: no report.** Run → dumps raw JSON `stress started`;
   HISTORICAL REPORTS stays "No stress tests run yet", status running:false.
   Persist + list a report; verify the `ws://localhost:8080` target.
10. **[SYS/DASH] Risk dashboard vs Lookup contradict.** Dashboard: user 1 flat
    $0.00, 0 accounts-with-positions, "no fill data"; Lookup: large long + PnL.
    WAL FILL filter = "no WAL events yet" (zero fills) → Lookup reads STALE
    persisted positions with no backing fills. One source of truth; clear/reconcile
    persisted positions on reset.

## CONFUSING
11. **[DASH] Process count reported 4 ways:** nav `7/7`, pulse `6/6`, KEY METRICS
    `7/7`, STATS `7`, Verify `7/6 running` (impossible). Pick one definition of
    expected processes (maker in/out consistently).
12. **[DASH] Health gauge YELLOW at 7/7:** `75 YELLOW -25 (panic in logs)` with no
    findable panic. Don't dock for stale/benign matches, or link the line.
13. **[DASH] Maker "Last error" while running fine:** `gw: Cannot connect
    localhost:8080` shown though maker placed 30k+ orders. Clear on reconnect / timestamp.
14. **[DASH] Error count inconsistent + benign:** mini-bar `errs 358` vs KEY METRICS
    `343` (red) vs Logs "no errors"; dominated by gateway `Broken pipe` WRN. Reconcile
    counters; don't count broken-pipe WRN as errors.
15. **[DASH] Rejections as raw codes:** `reason=4` (ambiguous in code too). Map to words.
16. **[DASH] Quantities in 3 unit systems:** orders raw fixed-point, form human-ish,
    risk dashboard $, lookup raw i64. Convert to human units at display boundary.
17. **[DASH] Book stats present dead symbols as live:** BTC/ETH/SOL Bid/Ask/Orders
    shown though only me-pengu runs; ladders say [SYNTHETIC][STALE] but stats omit it.
18. **[SYS] Mark "running" AND "down":** running in process views but Verify SKIP
    "no index (mark down)", WAL 0 bytes, Risk INDEX 0. mark connects Binance but never
    produces/persists an index.
19. **[DASH] Risk funding panels contradictory + duplicated:** FUNDING RATES `+0 bps`
    vs FUNDING panel `40 bps`; FUNDING + LIQUIDATION QUEUE each appear twice. Dedupe.
20. **[DASH] Faults contradicts Recovery:** /faults says do net/WAL faults via manual
    iptables/hex, but /recovery has one-click buttons for exactly those. Merge/point.
21. **[DASH] Raw JSON leaks to UI:** Latency probe + Stress print raw `{...}`. Format.
22. **[DASH] Latency "GATEWAY ROUND-TRIP (LIVE)" never populates:** stays `-- -- -- --`
    despite "tracks gateway round-trip on every submission". Wire or remove claim.
23. **[DASH] Verify latency regression apples-to-oranges:** `Order ack RTT p99 18453us
    +36806% (baseline 50us)` compares proxy-path RTT vs internal 50µs budget. Label/compare like-for-like.
24. **[DASH] Docs abandons the design system:** /docs is blue mkdocs, no top nav,
    jarring. Wrap in shared layout or keep top nav.
25. **[DASH] Logs default = self-noise:** "all" flooded by dashboard's own GET access
    logs; blank time col for server rows; `·` level for stdout lines. Default-hide server
    access logs; fix empty time.
26. **[SYS] mktdata drops + slow WAL flush only in Logs:** continuous `WRN seq gap
    sym=10`; me-pengu `flush took 10-14ms` (>10ms target). Underlies #3. Surface a drops metric.
27. **[DASH] Order buttons unlabeled (1x/5x/20x/100x — of what?); latency in alarm-red.**

## POLISH
28. **[SYS] Archive WAL balloons:** 6.9→10.2 GB in ~7min from maker quote churn (no
    crosses). Drives #7/#26 + fills disk. Retention/rotation on archive.
29. **[SYS] High auto-restart counts on "healthy" cluster:** gw-0 11, me/risk/mktdata 7.
    Surface as instability.
30. **[DASH] Crate pages inconsistent:** some have demo, some none; spec refs
    non-clickable; path formatting differs; tagline "12 crates" vs 14.
31. **[DASH] CPU readings disagree** across Control vs Overview. Sampling unreliable.
32. **[DASH] Stale CTA copy:** "start maker for liquidity" / "place an order" shown
    while maker running.

## Verified WORKING (don't regress): topology node-click, per-source log filter, Cast
explainer, Latency proxy-vs-internal honesty, Verify inline reasons, crash→feed→auto-restart, maker start/stop.
