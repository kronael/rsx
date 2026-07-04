# TASKS — TUI speed tests (ordered, file-scoped)

Execute in order. Tasks are partitioned so each touches a **disjoint file
set** (they ship sequentially — one code-editing sub at a time per the
repo rule — but must not stomp each other). T1 is front-loaded: every
full-stack test depends on `WsConn` and on all deps landing in one Cargo
edit. Each task is sized for one sonnet.

Conventions for every task: single import per line; `function`/named-fn
(no inline `tokio::spawn` — mirror `quic.rs`'s `run_thread`/`run_client`);
`cargo check -p rsx-tui` first; run the acceptance command and paste the
tail. Do NOT modify `rsx-cast`. Do NOT add a gateway QUIC listener.

Reference files to read before starting: `rsx-tui/src/{conn,quic,wire,
app,input,render}.rs`, `rsx-tui/tests/{quic_test,play_test,render_test}.rs`,
`rsx-cli/src/bench_probe.rs` (JWT mint + RTT loop), `rsx-gateway/src/
records.rs` (webproto frame shapes), `scripts/demo-trade.sh` (boot +
seed), `rsx-playground/tests/live/conftest.py` (skip-if-down pattern),
`rsx-playground/server.py:4240` (`send_order_to_gateway`, the browser
leg), `specs/2/49-webproto.md` (`{N:[...]}`/U/F/E/H).

---

## T1 — `WsConn`: webproto-49 gateway transport + all sprint deps

**Files (exclusive):** `rsx-tui/src/ws.rs` (new), `rsx-tui/src/lib.rs`
(add `pub mod ws;` + `pub use ws::WsConn;`), `rsx-tui/Cargo.toml`,
`rsx-tui/tests/ws_test.rs` (new).

**Do:**
- New `WsConn` implementing `GatewayConn`, structurally mirroring
  `quic.rs`: a named background thread runs a single-worker tokio runtime;
  `submit` pushes `OrderReq` onto an unbounded channel; the async task
  writes a webproto **`{N:[...]}`** text frame and reads U/F/E/H frames,
  pushing `GwEvent`s onto a std mpsc `poll_event` drains. No inline spawn:
  named `run_thread` → `run_client`.
- Transport: `tokio-tungstenite` WS client to `RSX_GW_LISTEN`
  (`ws://127.0.0.1:8080` default). Send `Authorization: Bearer <JWT>`
  header on the handshake.
- JWT mint helper (HS256, aud `rsx-gateway`, iss `rsx-auth`, `jti`,
  `exp`+3600, secret from `RSX_GW_JWT_SECRET` default
  `"rsx-dev-secret-not-for-prod-padpad"`) — copy `bench_probe.rs:100-125`.
  Put it in `ws.rs` as `pub fn mint_jwt(user_id, secret) -> String` so
  `cluster.rs` (T2) reuses it.
- Wire mapping (cite `records.rs` + `specs/2/49-webproto.md`):
  `OrderReq{side,price,qty,tif}` → `{N:[sym,side,px,qty,cid,tif,ro,po]}`;
  incoming `U`→`Accepted`/`Done`, `F`→`Fill`, `E`→`Rejected`, `H`→ignore.
  Book/Trade/Position come from the public/query frames — map what the
  gateway actually sends; if a frame has no `GwEvent`, drop it (log once).
- **Add ALL sprint deps to `Cargo.toml` now** so later tasks stay
  file-disjoint: `tokio-tungstenite`, `jsonwebtoken` (or `hmac`+`sha2`+
  `base64` — match whatever `bench_probe` uses), and dev-deps
  `criterion`, `tempfile` if needed. Keep existing quinn/ratatui deps.
- `rsx-tui/tests/ws_test.rs`: loopback proof mirroring `quic_test.rs` —
  stand up a tungstenite server on `127.0.0.1:0`, connect a real
  `WsConn`, submit one order, assert the server received the `{N:[...]}`
  frame and the client folded the echoed `F`→`Fill`. No cluster.

**Depends on:** nothing. **Acceptance:** `cargo test -p rsx-tui --test
ws_test` green; `cargo clippy -p rsx-tui -- -D warnings` clean;
`cargo build -p rsx-tui`.

---

## T2 — Headless harness + cluster + timing support

**Files (exclusive):** `rsx-tui/tests/support/mod.rs`,
`rsx-tui/tests/support/harness.rs`, `rsx-tui/tests/support/cluster.rs`,
`rsx-tui/tests/support/timing.rs`, `rsx-tui/tests/support_smoke.rs` (a
tiny test that exercises the harness over `MockConn`).

**Do:**
- `harness.rs`: `TuiHarness` owning `App`, `Box<dyn GatewayConn>`,
  `Terminal<TestBackend>`. Methods: `new_mock()`, `new_with(conn)`,
  `feed_key`, `feed_str`, `tick` (drain+draw), `screen()->String`,
  `wait_for(pred, timeout)->Option<Duration>`, `assert_screen`,
  `assert_state`. Reuse `screen()` flattening + `type_digits` shape from
  `play_test.rs`.
- `cluster.rs`: `connect(user_id)->Option<WsConn>` (dial `RSX_GW_LISTEN`,
  return `None` if unreachable), reusing `ws::mint_jwt`; `seed_book(conn)`
  posting a resting maker then returning (mirror `demo-trade.sh` order
  params); a `skip_if_no_cluster!` macro/helper that `eprintln!`s and
  early-returns.
- `timing.rs`: `pctile`, `min`, `mean`, warmup/measure loop helper, a
  `Timer` around submit→`wait_for(Fill)`, and a `to_json`/`table_row`
  emitter for the report.
- `support_smoke.rs`: drives `TuiHarness::new_mock()` through a scripted
  session (types an order, folds a `MockConn` `Fill`, asserts screen) to
  prove the harness compiles and works without a cluster.

**Depends on:** T1 (`WsConn`, `mint_jwt`). **Acceptance:** `cargo test -p
rsx-tui --test support_smoke` green; clippy clean.

---

## T3 — Ported trading e2e: orders + book

**Files (exclusive):** `rsx-tui/tests/e2e_orders.rs`,
`rsx-tui/tests/e2e_book.rs`.

**Do (each test env-gated, skip-if-no-cluster, over `WsConn`):**
- `e2e_orders.rs`: `submit_gtc_rests` (Accepted → `open_orders==1`),
  `submit_ioc_fills` (after `seed_book`, crossing IOC → `Fill` then
  `Done`, `fills==1`/`open_orders==0`), `invalid_order_rejected` (empty
  qty not sent via MockConn; malformed-sent → `Rejected` over real gw),
  `order_lifecycle_accepted_then_done` (assert order `Accepted`→`Fill`→
  `Done`; invariant "Fills precede ORDER_DONE").
- `e2e_book.rs`: `book_shows_bbo_after_maker` (after `seed_book`, screen
  shows best bid/ask + spread; `App` not crossed), `trade_tape_updates_on_
  fill` (`trades` grows after a crossing order).

**Depends on:** T1, T2. **Acceptance:** with a minimal cluster up
(`./rsx-playground/playground start` + start-all minimal), `cargo test -p
rsx-tui --test e2e_orders --test e2e_book` passes; with no cluster it
skips cleanly (0 failures). Clippy clean.

---

## T4 — Ported trading e2e: positions + guarantees

**Files (exclusive):** `rsx-tui/tests/e2e_positions.rs`,
`rsx-tui/tests/e2e_guarantees.rs`.

**Do (same gating as T3):**
- `e2e_positions.rs`: `position_updates_after_fill` (`Position` folds →
  positions panel shows net/entry; upnl sign drives color — assert via
  screen + `App.positions`).
- `e2e_guarantees.rs`: `no_crossed_book` (ladder never crosses after a
  sequence of orders — invariant #6), `exactly_one_completion` (exactly
  one `Done` xor `Rejected` per oid — invariant #2), and a fill-durability
  check if reachable (submit → observe fill → confirm it persists; may
  reuse the playground `/api/verify` like `demo-trade.sh`).

**Depends on:** T1, T2. **Acceptance:** cluster-up run passes, no-cluster
skips; clippy clean.

---

## T5 — Speed benches (C1/C3) + RTT comparison (C2)

**Files (exclusive):** `rsx-tui/benches/render_diff.rs` (new; add a
`[[bench]]` stanza to `Cargo.toml` — this is the **only** later touch to
Cargo.toml, keep it to the bench stanza), `rsx-tui/tests/e2e_latency.rs`,
`scripts/tui-bench.sh`.

> Note: T5 adds a `[[bench]]` entry to `rsx-tui/Cargo.toml`. This is a
> known, minimal, append-only exception to T1's "all Cargo edits in T1" —
> the bench target name isn't known until now. Append only; do not touch
> the `[dependencies]` T1 wrote.

**Do:**
- `render_diff.rs` (Criterion): bench C1 = `terminal.draw(draw)` after one
  `apply_event(Book|Trade|Fill)`; bench C3 = `handle_key(Enter)` on a
  filled form → `conn.submit` (MockConn). No cluster. Report p50/p99.
- `e2e_latency.rs` (cluster, gated): warmup+measure the submit→`Fill` RTT
  over `WsConn` using `timing.rs`; collect p50/p99/min; write a JSON blob
  to `./tmp/tui-rtt.json`. Optional stage scrape from the gateway log.
- `scripts/tui-bench.sh`: boot minimal cluster (playground API, like
  `demo-trade.sh`), `seed_book`, run the render bench + `e2e_latency`,
  pull the browser RTT from `GET /api/latency`, and print a
  browser-vs-TUI table + write the combined JSON for T6.

**Depends on:** T1, T2. **Acceptance:** `cargo bench -p rsx-tui
--bench render_diff` runs and prints C1/C3 numbers; `bash
scripts/tui-bench.sh` against a running cluster prints the comparison
table (or skips latency cleanly if no cluster); clippy clean.

---

## T6 — Report + Makefile targets + docs

**Files (exclusive):** `reports/20260704_tui-vs-browser-speed.md` (new),
`Makefile` (add `tui-e2e` + `tui-bench` targets only), `rsx-tui/README.md`
or `rsx-tui/ARCHITECTURE.md` (document the transports + test tiers),
`TESTING.md` (add the TUI tier row).

**Do:**
- Run `scripts/tui-bench.sh` against a live minimal cluster; fill the
  report from the real JSON. Report per `reports/` convention: what was
  measured, tables (metric | browser | TUI | delta) for C1/C3/C2 with the
  C2 shared-stack decomposition, conclusion, caveats (quiet box, p50/p99,
  closed-loop RTT, single host, shared backend), source bench + commit.
  Honest headline: client render/input 3–4 OOM cheaper + FastAPI relay
  removed from RTT; exchange backend identical.
- `Makefile`: `tui-e2e` (boot minimal via playground API, then `cargo
  test -p rsx-tui --test 'e2e_*'`), `tui-bench` (`bash
  scripts/tui-bench.sh`). Mirror `demo-trade`'s readiness poll.
- Docs: note `WsConn` (real gateway transport) vs `QuicConn` (transport
  bench only, gateway listener deferred) vs `MockConn` (unit); the test
  tiers table; the port-scope decision (INFRA specs out of scope by
  design).

**Depends on:** T5 (needs its numbers + script). **Acceptance:** `make
tui-bench` produces the report; `make tui-e2e` runs the ported suite
against a cluster; docs build/read clean.

---

## Ordering & dependency summary

```
T1 (WsConn + deps)  ──┬─> T2 (harness/cluster/timing) ──┬─> T3 (orders+book)
                      │                                  ├─> T4 (positions+guarantees)
                      │                                  └─> T5 (benches + RTT) ──> T6 (report+make+docs)
```

Ship strictly T1 → T2 → T3 → T4 → T5 → T6. File sets are disjoint (only
T1 writes `[dependencies]`/`lib.rs`; T5 appends one `[[bench]]` stanza).
T3 and T4 are independent of each other but ship sequentially per the
one-editor rule. Front-loaded T1 unblocks everything.
