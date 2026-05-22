# 17-REFINE-2 — Final Report

**Status: complete.** 2026-05-22.

This report covers the whole arc from the CTO+CEO review
through deployment, the maker/probe regressions surfaced by
that deployment, the deep fix to the FillRecord wire format
+ probe race, and the resulting **proven** per-leg latency
numbers.

---

## What we set out to do

The CTO + CEO dual-lens review (`.ship/16-CTO-CEO-REVIEW/
SYNTHESIS.md`) converged on a forced-rank refine list of
6 items, plus 3 latency probes deferred from the earlier
(a)/(b) decision. `.ship/17-REFINE-2/PLAN.md` bucketed them
into 4 disjoint scopes for parallel agents.

## What landed — the 9 items

### Bucket 1 — Rust correctness

- **F1.1** `cbfdb8d` Risk now spin-stall-drains on full
  fill/order rings; the silent `let _ = fill_prod.push(...)`
  that violated invariant #4 is gone. New regression test
  `fill_flood_position_matches_sum_of_fills` exercises it.
- **F1.2** `ee30c37` Workspace `let _ =` sweep: 19 sites → 0
  across rsx-risk, rsx-matching, rsx-marketdata, rsx-gateway,
  rsx-cli. MEMORY.md retracts the stale "0 violations" claim.
- **F1.3** `9159639` MAX_EVENTS 10_000 → 65_536; `emit()`
  asserts overflow per the spec invariant "ME never drops
  events." `event_buf` heap-boxed.

### Bucket 2 — Auth + JTI

- **F2.1-F2.3** `f4ff065` rsx-auth emits `jti` (uuid4 hex per
  token). Gateway `extract_user_and_record_jti` rejects tokens
  missing `jti` with 401 *before* touching the tracker — the
  replay defence is no longer null-defeatable. Replay
  regression test +
  `test_ws_handshake_rejects_missing_jti`.

### Bucket 3 — Playground UX truthfulness

- **F3.1** `605b317` i64 tick-size formatter on `/risk`,
  `/topology`, `/maker`. CEO-flagged
  `COLLATERAL 999999972019150` now renders as `$1,000,000.00`.
- **F3.2** `4aa554e` 1-second TTL cache on `/x/health`,
  `/x/key-metrics`, `/x/pulse`, `/x/cmp-flows` plus a bounded
  `_tail_lines` log scan. CEO repro:
  `curl /x/health` was 75 s, now < 200 ms.
- **F3.3** `4aa554e` Any `verify_results` FAIL forces health
  ≤ 49 (RED). The CEO's "100 GREEN while invariants fail"
  pattern is closed.
- **F3.4** `aec96ec` `/x/cmp-flows` reads three independent
  per-process WAL streams. The CEO's `1117/1117/1117` ghost
  is structurally impossible now.

### Bucket 4 — Probes + CI gate

- **F4.1** `59965c5` Native Rust `rsx-cli bench-probe`
  (tokio-tungstenite, jsonwebtoken inline mint) — for
  side-by-side timing against the Python probe.
- **F4.2** `f93ade1` `/api/latency-probe-gw` — submits an
  invalid order so gateway rejects fast-path without
  involving risk or ME. Isolates Python aiohttp overhead.
- **F4.3** `daccaba` Per-stage `tracing::info!(target =
  "latency", ...)` at 6 sites (later unified — see below).
- **F4.4** `5032085` `make bench-gate-e2e` + sealed
  `bench-reference.json` for CI regression detection.

## What broke when we deployed — and the deep fix

Bucket 2's hardening (gateway rejects tokens without `jti`)
correctly tightened auth but broke every JWT-minting client
that hadn't been updated. Six minutes after deploy, the maker
was in a circuit-breaker abort loop and every latency probe
returned 401.

### Sub-fix A: rsx-maker was dead code (`f7fce24` + `3b803ee`)

While diagnosing the maker outage, found that the Rust
rsx-maker binary was never on the live path — the playground
spawns `market_maker.py`. The Rust crate was a phantom. Drops
the workspace member; one less thing to be misled by.

### Sub-fix B: `jti` propagation (`2e97b4c` + `17057e0`)

Eight inline JWT mint sites didn't carry `jti`:
- `rsx-playground/market_maker.py` (the real maker)
- `rsx-playground/server.py` × 3 (latency probe, gw-only
  probe, internal user-WS proxy)
- `rsx-playground/stress_client.py` × 2
- `rsx-cli/src/bench_probe.rs` (with per-probe nonce so each
  handshake gets a fresh `jti`)
- `rsx-playground/tests/{conftest,stress_integration,
  acceptance}_test.py`

Added `"jti": uuid.uuid4().hex` (or the Rust nonce
equivalent) to every one. Maker came back up; probes started
landing — but with an even uglier symptom.

### Sub-fix C: probe race + FillRecord wire change (`82e9966` + `2fc3bac`)

Probes were returning `ok=false, skipped_fills=1, error=
"timeout waiting for fill"` even though the maker was
quoting + the ME was matching. Diagnosis chain:

1. Gateway log showed `write error conn N: Broken pipe` —
   *symptom*, not cause. The probe had already closed.
2. Per-stage trace showed `gateway_in → risk_in → me_in →
   me_out` all firing in ~150-500 µs. The exchange was fine.
3. The probe code required `probe_oid` (latched from `U` =
   order acknowledgement) BEFORE accepting any `F` (fill) as
   a match. But risk-side ordering is asynchronous — `U` and
   `F` take different paths through the response producer,
   and `F` can arrive first. The old F22 fix silently
   classified the probe's own `F` as "unrelated", sat until
   the 2 s deadline, then closed the WS.
4. After the close, the gateway tried to write the delayed
   final response and saw a closed socket — that was the
   "Broken pipe" log line.

The fix (`82e9966`): buffer `F` frames by `taker_oid` while
`probe_oid` is unknown; when `U` arrives, retro-match
against the buffer. Also surfaces `E` (order reject) frames
immediately — previously they were silently treated as
"unrelated" and the probe ran to its full timeout.

### Sub-fix D: 6-stage tracing actually unified (`2fc3bac`)

The F4.3 instrumentation had a subtle correctness bug:
`risk_out` and `gateway_out` anchored their `t0_ns` on
`fill.ts_ns` (ME's match timestamp), while `gateway_in /
risk_in / me_in / me_out` anchored on the order's
gateway-ingress timestamp. The per-stage deltas were not
composable — `risk_out` could *appear* lower than `me_in`.

Fix: wire change to `FillRecord`. Adds `taker_ts_ns: u64`
at offset 88 (fits in the existing 32 B of implicit
padding — `size_of::<FillRecord>()` stays 128). ME echoes
the taker order's `timestamp_ns` onto the fill. Risk and
gateway emit `risk_out` / `gateway_out` anchored on this
field (with a plausibility-guard fallback to `ts_ns` for
legacy WAL records).

Propagation reached 26 test/bench FillRecord literals; all
got `taker_ts_ns: 0` (test fixtures exercise structural
shape, not latency math). All 885 workspace tests still
pass.

## The proven numbers

After the fixes landed and the cluster redeployed, captured
N=500 latency probes + harvested per-stage traces.

### End-to-end (Python aiohttp probe)

| Metric | Value |
|--------|------:|
| e2e_us p50 | 11 875 µs |
| e2e_us p95 | 24 551 µs |
| e2e_us p99 | 342 893 µs |
| min | 10 428 µs |
| gw_only p50 | 301 µs |

### Per-stage (Rust path, 553 coherent traces)

| Stage | p50 µs | Δ from prev |
|-------|-------:|------------:|
| gateway_in | 0 | — |
| risk_in | 60 | 60 |
| me_in | 265 | 205 |
| me_out | 423 | 158 |
| risk_out | 462 | 39 |
| gateway_out | **1 128** | 666 |

### The decomposition

- **Rust GW→ME→GW p50 = 1.128 ms.**
- **Python aiohttp WS recv = ~10.7 ms** (11.875 - 1.128).
- The earlier "95% is Python" hypothesis was directionally
  right but slightly overstated. Actual ratio:
  **9.5% exchange / 90.5% probe client.**
- The 234× over-budget number the raw probe implied is **not
  the truth**. The real overage is **22×** (1.1 ms vs the
  50 µs design budget). Still substantial; not catastrophic.

### Where the biggest gap is

`risk_out → gateway_out` p50 = 666 µs (60% of the Rust
budget). That's the gateway's `push_to_user` + WS flush
path. Before this report it was effectively invisible —
without `taker_ts_ns`, the leg was credited at ~50 µs
(time since ME emit), not its real width. Next sprint
target: examine monoio's epoll → io_uring submission in
`push_to_user`. 667 µs to flush one WS frame is slow.

## Test + lint state

- `cargo test --workspace --lib --tests`: **885 passed / 0
  failed / 46 ignored** (+2 from refine-2 regression tests)
- `make lint`: clippy clean (-D warnings)
- `cd rsx-playground && uv run pytest tests/`: still 13
  pre-existing connection-refused failures in
  `stress_integration_test.py` (need gateway on port 28080
  that the playground doesn't start) — *not* introduced by
  this round
- Playwright: 459 tests across 24 spec files

## Commits in this round (since v0.2.0: 65 total)

```
a1e8358  bench: proven Rust GW→ME→GW p50 = 1.128ms (not 11.8ms; not 234x)
0355f2a  bench: e2e baseline post-probe-fix
2fc3bac  fix: FillRecord+Event::Fill carry taker_ts_ns; 6-stage unified
82e9966  fix: probe buffers F until U arrives; surface E (order reject)
2a237cd  docs: per-stage latencies (first measurement of the Rust path)
17057e0  fix: tests acceptance_test.py adds jti
2e97b4c  fix: add jti to every JWT mint site
3b803ee  chore: workspace drop rsx-maker member
01de656  docs: makefile + script refer to market_maker.py
f7fce24  chore: remove rsx-maker crate (dead — market_maker.py is the maker)
5bb3f35  refine: 17-REFINE-2 first-pass closeout
ee30c37  fix: sweep workspace let _ = drops
cbfdb8d  fix: risk stalls on full fill/order rings
9159639  fix: book bump MAX_EVENTS to 65536
5032085  fix: book raise MAX_EVENTS to 65536 (Bucket-4 sibling commit; race)
daccaba  bench: F4.3 per-stage latency tracing
aec96ec  fix: F3.4 per-pipe CMP counters from per-process WAL
4aa554e  fix: F3.2/F3.3 TTL cache + verify-fail → red health
f93ade1  bench: F4.2 gateway-only RTT probe
59965c5  bench: F4.1 native Rust E2E latency probe
f4ff065  fix: F2.1/F2.2/F2.3 emit jti + reject missing jti
605b317  fix: F3.1 i64 formatter for risk/topology/maker
3858e44  refine: 17-REFINE-2 plan synthesizing CTO+CEO review
```

## Known follow-ups (next sprint)

1. **Optimise `risk_out → gateway_out`** (666 µs p50).
   Profile the gateway's `push_to_user` + WS write path.
   Likely candidates: per-frame syscall to `epoll_ctl`,
   buffering vs immediate flush, monoio's io_uring batching
   granularity.
2. **Maker `order not found` noise.** Maker tries to cancel
   orders that have already been filled. Benign but loud;
   should silence those at the maker side or accept them
   server-side.
3. **Optimise `risk → ME` 205 µs.** Each side is doing one
   read/write syscall per datagram. Receive batching could
   halve this.
4. **Honest BLOG.md update.** The current "<50 µs design
   budget" framing should be paired with "22× over today,
   per the per-stage trace" so the narrative matches what
   we measure.
5. **Stress test integration**. The 13 pre-existing
   pytest failures need the gateway on port 28080. Either
   wire the playground to start that, or rewrite the tests
   to use the real port.

## What's NOT done

- The 234× over-budget Rust-internal optimisation is a
  multi-quarter project (kernel-bypass DPDK/AF_XDP swap),
  not this sprint.
- Trade UI "Loading..." forever — separate sprint, blocked
  on rsx-webui WS reconnect design.
- Publishing — repo policy forbids it (CLAUDE.md
  "Publishing"). Distribution is a founder decision.

## Methodology note

The dual-lens (CTO+CEO) DD with strict tool boundaries
(`meta/cto-ceo-review.md`) produced the strongest signal of
this whole audit cycle — both lenses converged on NO from
entirely different evidence. The deployment-then-deep-fix
arc was unplanned but exposed real bugs the static audit
couldn't have caught (the U-after-F race, the misaligned
`risk_out` anchor). Worth doing the live-cluster validation
step at the end of every refine sprint, not just claiming
"tests pass" as evidence.
