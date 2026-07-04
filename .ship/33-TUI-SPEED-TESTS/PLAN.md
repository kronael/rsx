# PLAN — TUI speed / usability e2e tests (rsx-tui vs browser)

Sprint `.ship/33-TUI-SPEED-TESTS`. Prove — with real measurements, not
marketing — that the native Rust terminal (`rsx-tui`) is dramatically
faster than the browser web UI (FastAPI playground dashboard) on the
keystroke → order → fill → on-screen path.

This is the design doc. Ordered, file-scoped shipping tasks are in
`TASKS.md`.

---

## 0. Ground truth (what the code actually is, 2026-07-04)

Read before trusting any framing. Several load-bearing facts differ from
the naive pitch.

**TUI (`rsx-tui/src/`).** Clean lib+bin split — the library is already
built to run headless:
- `app.rs` — `App` (pure state, public fields), `apply_event(GwEvent)`
  (the event fold), `drain(app, conn)` (pump transport once per tick),
  `Latency` + `lat_p50_ns`/`lat_min_ns`.
- `conn.rs` — `GatewayConn` trait (`submit(OrderReq)` + non-blocking
  `poll_event() -> Option<GwEvent>`), `MockConn` (scripted in-memory
  transport), `GwEvent` enum
  (`Connected/Book/Trade/Accepted/Fill/Done/Rejected/Position/Latency`).
- `input.rs` — `handle_key(App, KeyCode, conn) -> Control`.
- `render.rs` — `draw(Frame, App)`, pure over `&App`.
- `quic.rs` — `QuicConn` (quinn + tokio background thread, channel
  bridge) implementing `GatewayConn`.
- `wire.rs` — length-delimited **JSON** frame codec (`OrderReq` out,
  `GwEvent` in).

The seam for a real transport is the `GatewayConn` trait. Headless
driving already works via ratatui `TestBackend` (in-memory `Buffer` you
assert cell symbols on) + `MockConn`. Existing tests prove the pattern:
`tests/play_test.rs` (scripted key session), `tests/render_test.rs`
(buffer asserts), `tests/quic_test.rs` (loopback QUIC round-trip). 15
tests green today.

**The transport mismatch (central design fact).**
- `rsx-tui` speaks **QUIC** with a bespoke length-delimited **JSON**
  framing (`wire.rs`). Its own comments admit this is not byte-compatible
  with the gateway.
- The **gateway serves WebSocket only** — `ws.rs:35 ws_accept_loop` →
  `TcpListener::bind`, port `RSX_GW_LISTEN` default `0.0.0.0:8080`
  (`config.rs:98`). No QUIC anywhere in `rsx-gateway`. The wire format is
  webproto-49 tagged-JSON text frames `{N:[sym,side,px,qty,cid,tif,ro,po]}`
  in, `U`/`F`/`E`/`H` out (`records.rs`), decoded with
  `serde_json::from_str`.
- **The gateway QUIC listener is deferred product work** (diary
  2026-07-03 11:42: "QUIC listener DEFERRED … needs the running cluster
  to verify"). We do **not** build it in this sprint.

Consequence: to drive the *real* stack end-to-end from the TUI today, the
TUI needs a transport that speaks the gateway's real webproto WS. That is
task **T1 (`WsConn`)** below. This also matches the user's own framing
("connects directly to the gateway over the webproto WebSocket"). QUIC
stays an additive, transport-layer-only benchmark; it is not on the
full-stack path.

**`GwEvent::Latency` has no gateway producer.** The webproto `WsFrame`
enum has no `Latency` variant; the gateway only emits latency via
`latency_sample!` → `rsx_log` drain thread → tracing/log lines
(`handler.rs:438 gateway_in`, `route.rs:62-84 gateway_out`, etc.). The
TUI's `internal_ns`/`engine_ns` are demo/synthetic (`lib.rs:64`). So the
honest e2e round-trip number is a **client-side wall-clock RTT**
(submit → observed `Fill`), exactly how `bench_probe.rs` and the
browser's `send_order_to_gateway` already measure it. Server-side stage
attribution (net vs internal) is an optional log-scrape enhancement, not
the primary number.

**Auth.** Gateway WS handshake requires `Authorization: Bearer <JWT>`,
HS256, validated for `exp`/`nbf`/`aud=="rsx-gateway"`/`iss=="rsx-auth"` +
anti-replay `jti` (`jwt.rs:50`, `ws.rs:186`). Secret env
`RSX_GW_JWT_SECRET`, dev default `"rsx-dev-secret-not-for-prod-padpad"`.
Anyone with the secret mints a token trivially — `bench_probe.rs:100-125`
already does it in Rust; the harness copies that.

**No in-process spawnable entrypoint.** `rsx-gateway`, `rsx-risk`,
`rsx-matching` are binary-only: their `main()` wiring is private, `lib.rs`
exports only building-block modules. An in-process multi-tile harness
would have to duplicate three `main()`s. Rejected — see §2.

**Boot.** `/home/onvos/sandbox/rsx/start build_spawn_plan` launches
ME→Mark→Risk→Gateway→Marketdata→Recorder. Port bases: `BASE_ME_CAST=9100`,
`BASE_RISK_CAST=9200`, `BASE_GW_CAST=9300`, `BASE_GW_WS=8080`,
`BASE_ME_REPLICATION=9700`. Playground drives it via
`POST /api/processes/all/start?scenario=minimal` (~5-6 procs, ~20s).
Needs a real Postgres (external by default). `make tune-host` (rmem/wmem
25 MB) only needed for the auto-maker/stress path, not a bare minimal
boot. Closest existing harness pattern:
`rsx-playground/tests/live/` + `conftest.py` (assume cluster up, talk
real HTTP/WS, skip cleanly if unreachable) and `scripts/demo-trade.sh`
(boot minimal, post maker + IOC taker, assert fill in WAL).

**Browser comparison path.** Browser JS → FastAPI `:49171`
(`send_order_to_gateway`, `server.py:4240`: mints the *same* JWT, opens
aiohttp WS to the gateway, times µs) → gateway `:8080` → risk → ME → back
→ FastAPI → HTMX partial → DOM. `play_latency.spec.ts` + `/api/latency`
already measure this. The browser adds two hops the TUI removes: the
**FastAPI Python relay** and the **browser DOM/reflow**.

---

## 1. Thesis, decomposed honestly

"Dramatically faster" is really three independent claims. The stack
between gateway-in and gateway-out (risk + ME + casting/UDP) is **shared**
by both clients — we do not get to claim that as a TUI win. The TUI's real
advantages are the client edges:

| # | Claim | What the TUI removes | How we measure it | Expected scale |
|---|-------|----------------------|-------------------|----------------|
| C1 | **Client render** is orders of magnitude cheaper | browser DOM build + reflow + HTMX swap; replaced by a ratatui cell-diff | `TestBackend` render-diff time after one `apply_event` (Criterion) vs Playwright HTMX-partial swap time for the same book update | µs vs tens–hundreds of ms |
| C2 | **Client→gateway RTT** drops the Python relay | FastAPI aiohttp WS hop (`send_order_to_gateway`) | `WsConn` submit→`Fill` wall-clock vs browser→FastAPI→gateway RTT (`/api/latency`), **same gateway** | saves the FastAPI hop (~ms) |
| C3 | **Keystroke→submit** is a pure function call | JS event → XHR/HTMX → Python handler | in-proc `handle_key`→`conn.submit` timing (Criterion) | ns/µs vs ms |

C1 and C3 are deterministic, CPU-bound, and reproducible on any box (no
network). C2 is the one that shares the cross-process stack — the report
must **decompose** it: total RTT = (shared stack) + (client edge). The TUI
wins the client edge; we state the shared portion honestly rather than
banking it. The single strongest, most defensible headline is C1+C3 (the
client path), backed by C2's relay-elimination.

Non-goal: we do **not** claim the matching engine or casting transport is
faster because of the TUI — it is the identical backend.

---

## 2. Harness architecture

Three support layers, plus the transport (T1). All test-only except
`WsConn`, which is a legitimate TUI transport the product needs anyway.

### 2.1 Recommended stack bootstrap: **assume-running minimal cluster**

Chosen option (of the three the user posed):

- **(a) Playground `start-all minimal` / already-running cluster — CHOSEN.**
  The e2e tests connect to a cluster on the standard ports (`:8080` WS),
  mint a JWT, and **skip cleanly if the gateway is unreachable** (the
  `tests/live/` pattern). `make tui-e2e` brings the cluster up first (via
  the playground API, like `demo-trade.sh`) then runs the tests. This
  exercises the *real* cast/UDP + WS path, so C2's latency is real, and it
  hits the **same gateway** the browser probe hits → apples-to-apples.
  Cheapest reliable option that keeps the numbers honest.
- (b) Spawn real binaries per test — rejected: 6 processes, real PG
  dependency, port collisions, slow, flaky; no payoff over (a).
- (c) In-process harness linking risk/ME/gateway libs — rejected: none of
  the three expose a spawnable entrypoint (§0); we would reimplement three
  `main()`s and the casting wiring, and it would not exercise real UDP.

The unit/render/bench-C1/C3 layers need **no** cluster (MockConn +
TestBackend + Criterion) — they run in `make test`. Only the C2 full-stack
RTT and the ported trading e2e need the cluster, and they gate on it.

### 2.2 Headless TUI driver (`tests/support/harness.rs`)

The Playwright analog. A `TuiHarness` owning `App`, a boxed
`dyn GatewayConn`, and a `Terminal<TestBackend>`:

- `feed_key(KeyCode)` → `handle_key`.
- `feed_str(&str)` → digits one key at a time (see `type_digits` in
  `play_test.rs`).
- `tick()` → `drain(app, conn)` then `terminal.draw(draw)`.
- `screen() -> String` → flatten `backend().buffer().content()` cell
  symbols (already used in both existing test files).
- `wait_for(pred: Fn(&App) -> bool, timeout)` → loop `tick()` + small
  sleep until `pred` or deadline; returns elapsed. This is how e2e tests
  block for an async `Fill` to fold in. Also the C2 timer hook.
- `assert_screen(substr)` / `assert_state(pred)`.

Works identically over `MockConn` (unit) and `WsConn` (e2e) — the whole
point of the `GatewayConn` seam.

### 2.3 Cluster connect + seed (`tests/support/cluster.rs`)

- `mint_jwt(user_id)` → HS256 token, copy of `bench_probe.rs:100-125`
  (aud `rsx-gateway`, iss `rsx-auth`, `jti`, `exp`+3600), secret from
  `RSX_GW_JWT_SECRET` (default the dev secret).
- `connect(user_id) -> Option<WsConn>` → dial `RSX_GW_LISTEN`
  (default `ws://127.0.0.1:8080`), return `None` (→ test `eprintln!` +
  early-return skip) if the gateway is down.
- `seed_book(&mut WsConn)` → post a resting maker so a taker fills.
  Mirror `demo-trade.sh`: a resting limit (side buy @ px), then the test
  submits the crossing IOC. Use a distinct `user_id` for maker vs taker to
  avoid self-match rejection if the ME forbids it (check ME self-match
  behavior at implementation time; `demo-trade.sh` uses user 1 for both,
  so single-user is likely fine — verify).

### 2.4 Latency capture (`tests/support/timing.rs`)

- `Sample` collection + `pctile(&[u64], f64)` (p50/p99), `min`, `mean`.
- Warmup N iterations discarded, then M measured (mirror `bench_probe`).
- `Timer` = `Instant` around submit→`wait_for(Fill)`.
- Emit a compact JSON blob + a human table row for the report driver.
- Optional: `scrape_gateway_stage(oid)` — parse `latency_sample!` lines
  from the gateway log for `gateway_in`→`gateway_out` to attribute the
  shared-stack portion of C2. Nice-to-have; the primary number is
  client-side RTT.

---

## 3. Playwright → TUI port matrix

Inventory: **23 specs, 378 static `test(` cases**. Classification (full
per-spec table lives in the sprint notes / this section's source):

- **3 pure TRADING-FLOW** — `play_book`, `play_orders`, `play_risk`.
- **8 MIXED** — `play_maker`, `play_latency`, `play_guarantees`,
  `play_stress`, `play_safety`, `play_verify`, `play_down_contract`,
  `play_readiness` (only their order/book/fill halves are TUI-relevant).
- **12 INFRA/ORCHESTRATION** — `play_control`, `play_faults`,
  `play_health_truthful`, `play_infra`, `play_logs`, `play_navigation`,
  `play_orch`, `play_overview`, `play_session`, `play_topology`,
  `play_wal`, `play_walkthrough`.

**Scoping decision (not a gap):** the 12 INFRA specs test the *playground
dashboard* — process control, session lifecycle, topology graph, log
viewer, WAL inspector, nav. Those are cluster-console features. The TUI is
a **trading terminal**, not a cluster console; it deliberately has no such
surface. Porting them would mean inventing UI the TUI does not and should
not have. We port the trading surface only, and record the INFRA specs as
**out of scope by design** (ops tooling lives in the dashboard / `rsx-cli`,
per the trust-boundary "every concern has one owner" principle).

Port matrix (each backed by the real stack unless marked *unit*):

| Playwright spec (what it exercises) | TUI test | Asserts | Latency measured |
|---|---|---|---|
| `play_book` — ladder, BBO, symbol, live fills after order | `e2e_book::book_shows_bbo_after_maker` | screen shows best bid/ask + spread after `seed_book`; `App.bids/asks` sorted, no crossed book | book-update render-diff (C1) |
| `play_book` — fills tape updates | `e2e_book::trade_tape_updates_on_fill` | `trades` panel + `App.trades` grow after a crossing order | — |
| `play_orders` — submit limit (resting) | `e2e_orders::submit_gtc_rests` | type side/px/qty, Enter; `Accepted` folds → `open_orders==1`, status "accepted" | keystroke→submit (C3) |
| `play_orders` — submit IOC that fills | `e2e_orders::submit_ioc_fills` | after `seed_book`, crossing IOC yields `Fill` then `Done`; `fills==1`, `open_orders==0` | **submit→Fill RTT (C2)** |
| `play_orders` — invalid order rejected | `e2e_orders::invalid_order_rejected` | zero/empty qty → not sent (unit, `MockConn`); malformed-but-sent → `Rejected` status over real gw | — |
| `play_orders` — recent-orders / lifecycle trace | `e2e_orders::order_lifecycle_accepted_then_done` | ordered `Accepted`→`Fill`→`Done`; invariant "Fills precede ORDER_DONE" | — |
| `play_orders` — batch submit | `e2e_orders::batch_submit_throughput` | submit N orders back-to-back; all accepted | burst submit rate (C2, aux) |
| `play_risk` — positions after fills | `e2e_positions::position_updates_after_fill` | `Position` event folds → positions panel shows net/entry; upnl sign→color | position-update render-diff (C1) |
| `play_risk` — freeze/unfreeze, heatmap, funding | *out of scope* | admin/ops surface, not a trading-client concern | — |
| `play_maker` (trading half) — quotes → book/BBO | folded into `seed_book` + `e2e_book` | resting quotes populate the ladder | — |
| `play_latency` (trading half) — order-submit + e2e fill probe timing | `e2e_latency::submit_fill_rtt` + `bench render_diff` | RTT p50/p99 recorded; render-diff µs recorded | **C1 + C2 (the headline)** |
| `play_latency` (browser half) — per-tab page-load ms | *comparison input, not ported* | reuse the browser numbers as the "before" column | browser page/HTMX ms |
| `play_guarantees` (trading half) — no crossed book, exactly-one completion, fill durability | `e2e_guarantees::no_crossed_book` + `exactly_one_completion` | ladder never crosses; exactly one `Done` xor `Rejected` per oid | — |
| `play_stress` (trading half) — load run | `e2e_latency::burst_submit_throughput` (shared w/ orders) | sustained submit; no dropped completions | throughput |
| INFRA ×12 | *out of scope by design* | dashboard/ops surface the TUI intentionally lacks | — |

Unit-only pieces (no cluster, keep in `make test`): the event-fold
correctness (`apply_event`), the render-diff assertions, keystroke
editing, incomplete/rejected form handling. These extend the existing
`play_test.rs`/`render_test.rs`; do not duplicate them.

---

## 4. Speed-measurement methodology

**Layers timed**
1. **C3 keystroke→submit** (in-proc, Criterion): `handle_key(Enter)` on a
   filled form → `conn.submit` returns. ns.
2. **C1 render-diff** (in-proc, Criterion): time `terminal.draw(draw)`
   after a single `apply_event(Book|Trade|Fill)` — the incremental cost of
   painting one update. Compare against browser HTMX partial swap time for
   the equivalent "book changed" update (Playwright `page.waitForResponse`
   / swap-settled timing from `test_helpers.ts`).
3. **C2 submit→Fill RTT** (full stack, wall-clock): `WsConn.submit`
   Instant → `wait_for(|a| a.fills > 0)`. Compare against
   `/api/latency` (browser→FastAPI→gateway RTT) — **same gateway**.

**Honest-numbers discipline**
- **Warmup**: discard first N (JIT-free but TLS/connection/PG cache warm,
  first-touch page faults). Mirror `bench_probe`.
- **Percentiles**: report p50 **and** p99 (and min), never mean-only —
  tail is the story on a non-isolated box.
- **Quiet-box caveat**: cores are not pinned/isolated for these tests;
  tail noise is real. State it. The C1/C3 numbers are CPU-bound and stable;
  the C2 tail carries scheduler noise shared by both clients.
- **Open vs closed loop**: RTT is closed-loop single-order (one in flight);
  throughput/burst is a separate open-loop number. Never conflate.
- **Same gateway, same order, same symbol, same JWT scheme** for both
  clients — the only difference is the client path. That is the entire
  validity of the comparison.
- **Decompose C2**: total RTT − shared-stack (from gateway
  `gateway_in`→`gateway_out` log stamps, if scraped) = client edge. Report
  the edge as the TUI win; report the shared stack as shared.

**Comparison design (TUI vs browser)**
- Browser column: existing `/api/latency` (order RTT) + `play_latency`
  page/HTMX timings. No new browser code — reuse.
- TUI column: `e2e_latency` RTT + `render_diff` bench.
- Both to the **same running gateway** (the C2 requirement). Run them in
  the same session against the same cluster so the shared stack is
  identical.

---

## 5. "Supersedes the browser" artifact

- **Reproducible bench driver**: `scripts/tui-bench.sh` — boot minimal
  cluster (playground API, like `demo-trade.sh`), `seed_book`, run the TUI
  RTT + render-diff, pull the browser numbers from `/api/latency`, emit a
  comparison table + JSON.
- **Report**: `reports/20260704_tui-vs-browser-speed.md` (per the
  `reports/` convention: what was measured, numbers as tables, conclusion,
  caveats, source bench + commit). Columns: metric | browser | TUI |
  delta. Rows: keystroke→submit (C3), render one update (C1), order→fill
  RTT (C2, with shared-stack decomposition). The honest headline: the TUI
  client path is 3–4 orders of magnitude cheaper on render/input and
  removes the FastAPI relay from the RTT; the shared exchange backend is
  identical.
- **Make targets**: `make tui-bench` (driver + report), `make tui-e2e`
  (full-stack ported tests, needs cluster).

---

## 6. Test taxonomy & placement

| Tier | Where | Runner | Cluster? |
|---|---|---|---|
| Unit — event-fold, form edit, guarantees on folded state | extend `rsx-tui/tests/play_test.rs`, `src/*_test.rs` | `make test` | no |
| Render — buffer asserts | extend `rsx-tui/tests/render_test.rs` | `make test` | no |
| Transport unit — `WsConn` loopback | `rsx-tui/tests/ws_test.rs` (mirror `quic_test.rs`) | `make test` | no |
| Bench C1/C3 — render-diff, keystroke→submit | `rsx-tui/benches/render_diff.rs` (Criterion) | `make perf` / `make tui-bench` | no |
| Full-stack e2e — ported trading flows | `rsx-tui/tests/e2e_*.rs`, env-gated / `#[ignore]` | `make tui-e2e` | yes (skip if down) |
| Bench C2 — RTT comparison | `rsx-tui/tests/e2e_latency.rs` + `scripts/tui-bench.sh` | `make tui-bench` | yes |

`make e2e` already runs `cargo test --workspace --test '*'`, so any
non-ignored `rsx-tui/tests/*.rs` are picked up automatically. Full-stack
tests use env-gate + skip (not `#[ignore]`) so they *attempt* to run under
`make tui-e2e` but no-op cleanly elsewhere — matching `tests/live/`.

---

## 7. Risks / unknowns & mitigations

1. **No gateway QUIC listener; TUI's QUIC ≠ gateway wire.** → Build
   `WsConn` (webproto-49 over WS), the transport the full-stack path
   actually needs and the user's framing assumes. QUIC stays a
   transport-layer-only bench (loopback QUIC vs loopback WS). **Do NOT add
   a gateway QUIC listener in this sprint.**
2. **No `GwEvent::Latency` from the gateway.** → Primary RTT is
   client-side wall-clock (as `bench_probe`/browser already do). Stage
   attribution via optional log scrape only. Do not fake `internal_ns`.
3. **Cluster bootstrap flakiness / real Postgres dependency.** → Tests
   skip cleanly when the gateway is unreachable (the `tests/live/`
   pattern); `make tui-e2e` boots via the playground API with the same
   readiness poll `demo-trade.sh` uses (`≥6 running`, deadline). Never
   fail a unit run because a cluster is absent.
4. **JWT / auth in tests.** → Mint in-Rust HS256 with the dev secret,
   copying `bench_probe.rs`. `jti` required. No bypass, none needed.
5. **Fills need resting liquidity.** → `seed_book` posts a maker before
   the taker (mirror `demo-trade.sh`). Verify self-match policy; use two
   `user_id`s if the ME rejects self-match.
6. **Non-isolated cores → tail noise; `make tune-host` for bursts.** →
   Report p50+p99, state the quiet-box caveat, keep C1/C3 (CPU-bound,
   stable) as the defensible headline. Run `tune-host` before the burst
   number if the maker/stress path is exercised.
7. **Apples-to-apples validity.** → Both clients hit the **same gateway**
   in the same session; decompose the shared stack out of C2 so the claim
   is the client edge, not the shared backend. No mean-only, no
   cherry-picked min.
8. **Overclaiming.** → The report explicitly separates client-edge wins
   (real, large) from the shared backend (identical). "Dramatically
   faster" is scoped to render + input + relay-elimination.

---

## 8. Hard constraints

- **Do NOT modify `rsx-cast`** — frozen transport crate.
- **Do NOT add a gateway QUIC listener** — deferred product work, out of
  scope here.
- `WsConn` is additive to `rsx-tui`; casting untouched.
- Single import per line, `function`/named-fn conventions, no inline
  `tokio::spawn` (mirror `quic.rs`'s named `run_thread`/`run_client`).
- All new deps land in one Cargo edit (T1) so later tasks stay
  file-disjoint.
