# Sleep / Timeout Audit — 2026-05-23

Scope: every `sleep`, `Duration::from_*`, `timeout`, `wait_for`,
`tokio::time::*`, `monoio::time::*`, `thread::sleep`, and timer-driven
`tick()` in `rsx-*/src/` (Rust). Tests and benches under `tests/`,
`benches/`, and `*_test.rs` are excluded per the audit brief.

Two known prod bugs in this category are the entry point:
- `rsx-gateway/src/main.rs:407` — `monoio::time::sleep(100µs)` added
  ~655 µs to GW→ME→GW p50. The earlier patch was reverted; the real
  fix is monoio `UdpSocket` in gateway (caller owns the socket —
  rsx-dxs stays runtime-free).
- `rsx-marketdata/src/main.rs:328` — identical pattern.

Oracle (codex) was consulted on the monoio sleep-as-yield class: the
monoio timer wheel has 1 ms granularity, so `sleep(Duration::from_micros(100))`
**rounds up to ~1 ms**. The cheapest tactical fix is
`monoio::task::yield_now()`; the right end state is gateway/marketdata
owning `monoio::net::UdpSocket` and awaiting readiness.

---

## 1. Summary table

Path: HOT = per-order critical path. WARM = per-event but
amortised. COLD = startup, shutdown, error retry, reconnection,
background bookkeeping.

| file:line | path | classification | impact | proposed fix |
|---|---|---|---|---|
| rsx-gateway/src/main.rs:407 | HOT | QUESTIONABLE | ~655 µs / cycle on GW→ME→GW p50 (timer wheel rounds 100 µs up to ~1 ms) | `monoio::task::yield_now()` short-term; gateway owns `monoio::net::UdpSocket` long-term (rsx-dxs stays runtime-free) |
| rsx-marketdata/src/main.rs:328 | WARM | QUESTIONABLE | identical pattern; mostly off the order critical path but on BBO/fill propagation to subscribers (~1 ms staleness) | same as above |
| rsx-gateway/src/handler.rs:166 | WARM | NEEDED | per-WS-connection; 10 ms upper bound on outbound-drain wakeup. Not on order ingress (which is read-driven). | leave — explicit cap on outbound drain latency, comment cites io_uring cancel-safety |
| rsx-dxs/src/cmp.rs:154 (CmpSender heartbeat_interval) | WARM | NEEDED | timer-driven heartbeat; default 10 ms cadence per `CmpConfig`. One small UDP send per tick. | leave — needed for liveness |
| rsx-dxs/src/cmp.rs:528 (last_drop_warn) | COLD | NEEDED | log throttle for unsupported-version drops, 60 s seed | leave |
| rsx-dxs/src/cmp.rs:536 (CmpReceiver status_interval) | WARM | NEEDED | timer-driven flow-control STATUS_MESSAGE; default 10 ms | leave |
| rsx-dxs/src/cmp.rs:578 (drop-warn rate limit) | COLD | NEEDED | 5 s log throttle, not on hot path | leave |
| rsx-dxs/src/client.rs:72 (tip_persist_interval) | WARM | NEEDED | every 10 ms, persists DXS tip to disk during replay | leave |
| rsx-dxs/src/client.rs:122-136 (replay reconnect backoff) | COLD | NEEDED | exponential backoff with ±20 % jitter on stream errors | leave |
| rsx-dxs/src/wal.rs:165 (`elapsed > 10ms` flush warn) | COLD | NEEDED | structured-log diagnostic — sets `flush_stalled` flag | leave |
| rsx-dxs/src/config.rs:44 (heartbeat_interval_ms default 10) | n/a | NEEDED | default for CmpSender/Receiver | leave |
| rsx-dxs/src/config.rs:45 (status_interval_ms default 10) | n/a | NEEDED | default for STATUS_MESSAGE | leave |
| rsx-gateway/src/state.rs:63 (circuit cooldown) | COLD | NEEDED | circuit breaker open-state duration | leave |
| rsx-gateway/src/state.rs:225 `should_send_heartbeat` | WARM | NEEDED | interval check, no actual sleep — just comparison | leave |
| rsx-gateway/src/state.rs:254 `is_heartbeat_timeout` | WARM | NEEDED | interval check, no sleep | leave |
| rsx-gateway/src/config.rs:7 `order_timeout_ms` | n/a | NEEDED | order-pending eviction threshold | leave |
| rsx-gateway/src/config.rs:8 `heartbeat_interval_ms` | n/a | NEEDED | WS heartbeat cadence | leave |
| rsx-gateway/src/config.rs:9 `heartbeat_timeout_ms` | n/a | NEEDED | WS idle reap threshold | leave |
| rsx-gateway/src/main.rs:35 `PENDING_SWEEP_INTERVAL_US` (100 ms) | WARM | NEEDED | pending-orders GC; gated by interval check, not a sleep | leave |
| rsx-gateway/src/main.rs:363 `order_timeout_ms * NS_PER_MS` | WARM | NEEDED | derived cutoff for sweep | leave |
| rsx-gateway/src/main.rs:384 `heartbeat_timeout_ns` | WARM | NEEDED | derived cutoff for stale-conn reap | leave |
| rsx-gateway/src/handler.rs:115 `is_heartbeat_timeout` call | WARM | NEEDED | check per connection | leave |
| rsx-marketdata/src/main.rs:206 `heartbeat_timeout_ns` | WARM | NEEDED | derived cutoff | leave |
| rsx-marketdata/src/main.rs:309 hb interval check | WARM | NEEDED | broadcast heartbeats every 5 s default | leave |
| rsx-marketdata/src/main.rs:315 timeout-check interval | WARM | NEEDED | reap WS conns that stopped heartbeating | leave |
| rsx-marketdata/src/state.rs:331 `check_timeouts` | WARM | NEEDED | iterator over connections, no sleep | leave |
| rsx-marketdata/src/config.rs:18 `heartbeat_timeout_ms` | n/a | NEEDED | default 10 s × 1000 | leave |
| rsx-marketdata/src/config.rs:99 `heartbeat_interval_ms` | n/a | NEEDED | default 5 s × 1000 | leave |
| rsx-matching/src/dedup.rs:6 `DEDUP_WINDOW` 300 s | n/a | NEEDED | dedup retention window per spec | leave |
| rsx-matching/src/dedup.rs:8 `DEDUP_CLEANUP_INTERVAL` 10 s | WARM | NEEDED | cleanup cadence on ME loop | leave |
| rsx-matching/src/wal_integration.rs:187 `flush_if_due` (10 ms) | WARM | NEEDED | WAL flush cadence on ME hot loop | leave |
| rsx-mark/src/main.rs:26 `FLUSH_INTERVAL` 10 ms | WARM | NEEDED | WAL flush cadence | leave |
| rsx-mark/src/main.rs:28 `SWEEP_INTERVAL` 1 s | WARM | NEEDED | staleness sweep | leave |
| rsx-mark/src/main.rs:279 restart-on-crash sleep 5 s | COLD | NEEDED | restart backoff after crash | leave |
| rsx-mark/src/source.rs:158 reconnect backoff (exp + jitter) | COLD | NEEDED | upstream WS reconnect | leave |
| rsx-risk/src/persist.rs:376 `FLUSH_INTERVAL_MS` 10 ms | WARM | NEEDED | PG write-behind cadence; runs on dedicated tokio task, **not on shard order path** (see §4 below) | leave |
| rsx-risk/src/persist.rs:378 `BACKOFF_INIT_MS` 100 ms | COLD | NEEDED | PG retry initial backoff | leave |
| rsx-risk/src/persist.rs:380 `BACKOFF_MAX_MS` 30 s | COLD | NEEDED | PG retry cap | leave |
| rsx-risk/src/persist.rs:417 main flush sleep | COLD | NEEDED | drives the PG flush loop; off shard path | leave |
| rsx-risk/src/persist.rs:492 backoff sleep on error | COLD | NEEDED | PG retry sleep | leave |
| rsx-risk/src/main.rs:55 `RESTART_BACKOFF_SECS` 5..60 | COLD | NEEDED | shard crash-restart schedule | leave |
| rsx-risk/src/main.rs:216 restart sleep | COLD | NEEDED | crash-restart with jitter | leave |
| rsx-risk/src/main.rs:1052 lease_renew_interval | COLD | NEEDED | PG advisory lock renewal ~1 s | leave |
| rsx-risk/src/main.rs:1120 stop_persist watchdog 5 s cap | COLD | NEEDED | bounded wait for persist worker to exit | leave |
| rsx-risk/src/main.rs:1128 watchdog poll 50 ms | COLD | NEEDED | watchdog tick during demote drain | leave |
| rsx-risk/src/main.rs:1312 lease_poll_interval | COLD | NEEDED | replica polls advisory lock acquisition | leave |
| rsx-risk/src/config.rs:32 `lease_poll_interval_ms` 500 | n/a | NEEDED | default | leave |
| rsx-risk/src/config.rs:33 `lease_renew_interval_ms` 1000 | n/a | NEEDED | default | leave |
| rsx-risk/src/funding.rs:4 funding `interval_secs` | n/a | NEEDED | funding settlement cadence | leave |
| rsx-cli/src/main.rs:828 `dump_follow` EOF poll 100 ms | COLD | NEEDED | tail-follow loop for `rsxcli` operator tool | leave |
| rsx-cli/src/main.rs:838 reopen retry 100 ms | COLD | NEEDED | same loop, error retry | leave |
| rsx-cli/src/main.rs:906 follow-json EOF poll 100 ms | COLD | NEEDED | same as 828, JSON variant | leave |
| rsx-cli/src/main.rs:916 follow-json reopen retry | COLD | NEEDED | same | leave |
| rsx-cli/src/bench_probe.rs:74 `timeout_s` 2.0 | n/a (bench) | out of scope | per-probe timeout (operator-driven, not production traffic) | leave |
| rsx-cli/src/bench_probe.rs:236 `timeout: Duration` | n/a (bench) | out of scope | parameter | leave |
| rsx-cli/src/bench_probe.rs:291 deadline | n/a (bench) | out of scope | per-probe deadline | leave |
| rsx-cli/src/bench_probe.rs:299 `tokio::time::timeout` recv | n/a (bench) | out of scope | bounded WS recv | leave |
| rsx-cli/src/bench_probe.rs:368 `Duration::from_secs_f64` | n/a (bench) | out of scope | parse arg | leave |
| rsx-log/src/lib.rs:165 drainer interval | WARM | NEEDED | configurable drain cadence for structured-log metrics ring | leave |
| rsx-log/src/lib.rs:173 drainer `thread::sleep(interval)` | WARM | NEEDED | drain loop body | leave |

Site count: **57** distinct sites catalogued
(2 HOT/WARM QUESTIONABLE, the rest NEEDED or out-of-scope).

---

## 2. HOT path — fix candidates ranked

### #1 — `rsx-gateway/src/main.rs:407` (the known one)

```
            // Yield to monoio scheduler
            monoio::time::sleep(
                std::time::Duration::from_micros(100),
            )
            .await;
```

This is the inner loop of the gateway's CMP-from-risk poll. Each
trip:
1. drains `cmp_receiver.try_recv()` (CMP/UDP fills + responses from
   Risk),
2. calls `sender.tick()` / `cmp_receiver.tick()` /
   `sender.recv_control()`,
3. sweeps pending orders,
4. broadcasts server heartbeat,
5. reaps idle connections,
6. **sleeps 100 µs as a yield**.

Measured impact: ~655 µs added to GW→ME→GW p50 (per LOG.md / the
prior reverted patch). Mechanism per oracle (codex): monoio's
timing wheel has 1 ms slots, so a 100 µs sleep rounds up — the
sleep is "fundamentally the wrong tool for a microsecond-scale
yield."

Fix path:
- **Tactical (1 line):** swap for `monoio::task::yield_now().await`.
  Removes the timer-wheel tax. Loop becomes a cooperative
  busy-poll: idle CPU goes up but order-path latency drops by ~the
  full sleep cost.
- **Right answer (~50 lines):** gateway owns the `UdpSocket`
  (rsx-dxs is runtime-free by design). Replace
  `std::net::UdpSocket` with `monoio::net::UdpSocket` in
  gateway's main loop and `select!` over (a) socket readable,
  (b) a 1 ms heartbeat/sweep timer. rsx-dxs itself never gains
  a runtime dep — caller passes bytes in, gets records out.

Caveat: the tactical fix needs the dedicated-core picture to be
already true for the gateway tile (CLAUDE.md says gateway is on
its own pinned thread). If it isn't, idle CPU will be a problem.

### #2 — `rsx-marketdata/src/main.rs:328`

Same pattern, same fix, lower priority because marketdata is not
on the GW→ME→GW critical path. The 1 ms staleness shows up in
BBO/fill propagation to L2 subscribers. Out of scope for the
production-latency p50 number; in scope for "tick-to-WS" tail.

### Nothing else on HOT path.

All matching, risk-shard, and CMP `tick()` calls are interval
checks (compare `now` against `last_*`), not sleeps. They cost a
load and compare each loop iteration — sub-µs.

---

## 3. WARM path

WARM-path sites are timer-driven bookkeeping that runs on every
loop iteration but does work only when the cadence elapses. These
are correctly designed as comparisons + occasional action:

- `flush_if_due` in matching, mark, dxs/wal: 10 ms WAL flush.
  NEEDED for durability; lower bound on fsync wakeup is the disk
  itself, not us.
- `should_send_heartbeat` / `is_heartbeat_timeout` in gateway,
  marketdata, dxs/cmp: derived nanosecond cutoffs, no actual sleep
  involved.
- `CmpSender::tick` (cmp.rs:239) and `CmpReceiver::tick`
  (cmp.rs:799): heartbeat (10 ms) and status (10 ms) timers. Cost
  per call is a `Instant::now()` and one compare; action (UDP
  send) only when interval elapses.
- `DEDUP_CLEANUP_INTERVAL` 10 s on the matching loop: cheap.
- `BOOK_TTL_NS` 60 s on marketdata: cheap.

The only WARM sites that are problematic are #1 and #2 above —
because the *yield* mechanism is wrong, not because the cadence
is.

---

## 4. COLD path

All correctly classified, all NEEDED:

- `rsx-mark/src/main.rs:279` — restart loop 5 s.
- `rsx-mark/src/source.rs:158` — exponential reconnect backoff.
- `rsx-dxs/src/client.rs:122` — replay reconnect backoff.
- `rsx-risk/src/persist.rs:417,492` — PG write-behind cadence and
  retry sleep. **Important:** this runs on `tokio::spawn` inside a
  separate `current_thread` runtime on its own OS thread (see
  `rsx-risk/src/main.rs:318` — `std::thread::spawn(move || { rt =
  Builder::new_current_thread().build()...; rt.block_on(run_persist_worker_...) })`).
  The shard's order-processing loop is a different thread driven
  by `shard.run_once()`; the persist worker pops from an `rtrb`
  consumer fed by the shard. So the `tokio::time::sleep(10ms)` in
  persist does **not** sit on the order path. Verified by
  inspecting `rsx-risk/src/main.rs` around lines 318–348.
- `rsx-risk/src/main.rs:216` — shard crash-restart 5..60 s with
  ±20 % jitter.
- `rsx-risk/src/main.rs:1052,1311` — PG advisory-lock renewal
  (~1 s) and poll (500 ms) for HA promotion. Out of order path.
- `rsx-risk/src/main.rs:1120,1128` — bounded watchdog wait on
  persist-worker shutdown. Off path.
- `rsx-gateway/src/state.rs:63` — circuit-breaker cooldown.
- `rsx-cli/src/main.rs:828,838,906,916` — operator tool
  (`rsxcli`), follow-mode WAL tail-poll. Not in any production
  process.
- `rsx-log/src/lib.rs:165,173` — structured-log drainer thread.
  Configurable interval; runs out-of-band; OK.

---

## 5. Systemic patterns

**Sleep-as-yield (replace with readiness-driven I/O): 2 sites.**
- `rsx-gateway/src/main.rs:407`
- `rsx-marketdata/src/main.rs:328`

This is the **only** systemic class with a production-latency
impact. Both share the exact same shape: the loop polls a sync
`std::net::UdpSocket` via `try_recv()`, does bookkeeping, then
sleeps 100 µs. Oracle (codex, 2026-05-23) confirms the monoio
timer wheel granularity is the root cause — at 1 ms slots, a
100 µs sleep is a 10× overrun in expectation. Tactical fix
(`yield_now`) is one line each. Strategic fix is monoio
`UdpSocket` in gateway/marketdata callers (~50 LOC each) —
rsx-dxs is runtime-free; no changes to `CmpReceiver`/`CmpSender`.

**Backoff-based retry (correctly designed): 5 sites.**
- `rsx-mark/src/main.rs:279` (restart)
- `rsx-mark/src/source.rs:158` (WS reconnect)
- `rsx-dxs/src/client.rs:122` (DXS reconnect)
- `rsx-risk/src/main.rs:216` (shard restart)
- `rsx-risk/src/persist.rs:492` (PG retry)

All use exponential or table-driven schedules with jitter.

**Genuine timer-driven (heartbeat, sweep, flush, cleanup): ~20 sites.**
- `CmpSender::tick` heartbeat (dxs/cmp.rs:239, 10 ms default,
  `RSX_*_HEARTBEAT_INTERVAL_MS`)
- `CmpReceiver::tick` status (dxs/cmp.rs:799, 10 ms default,
  `RSX_*_STATUS_INTERVAL_MS`)
- `flush_if_due` in matching, mark, dxs/wal (10 ms WAL flush
  cadence)
- WS heartbeat in gateway + marketdata (5 s default,
  10 s timeout)
- pending-orders sweep in gateway (100 ms)
- snapshot save in matching (10 s)
- dedup cleanup in matching (10 s window, 300 s retention)
- mark sweep (1 s)
- book TTL eviction in marketdata (60 s)
- DXS tip persist in client (10 ms)
- persist-worker flush in risk (10 ms)
- lease renew / poll in risk main (1 s / 500 ms)
- rsx-log drainer (configurable, default 100 ms)

All correctly implemented as `if now - last >= interval` checks
inside busy/poll loops, not as `sleep().await`. They cost one
`Instant::now()` + one compare per loop trip.

**Configuration knobs (no runtime cost): ~12 sites.**
Defaults in `CmpConfig`, `GatewayConfig`, `MarketdataConfig`,
`RiskConfig`. Env-overridable. Out of scope for latency, in scope
for ops.

---

## 6. Forced-rank: top 3 to fix this sprint

1. **`rsx-gateway/src/main.rs:407` — `monoio::time::sleep(100µs)`
   in the CMP poll loop.** Measured +~655 µs on GW→ME→GW p50.
   Tactical fix (yield_now) is 1 line; strategic fix (monoio
   UdpSocket) is the right end state. **Highest ROI in the entire
   workspace.**

2. **`rsx-marketdata/src/main.rs:328` — identical pattern.** Not
   on the order critical path; on the BBO/fill propagation path
   to WS subscribers. Same fix as #1, ~50 LOC for the strategic
   variant. Lower priority than #1 but same code change.

3. *(No real #3 in this class.)* All other sleeps are either
   genuinely timer-driven or off the production-traffic path.
   Honorable mention: **revisit the WARM-path timing wheel for
   `flush_if_due` and `CmpSender::tick` consumers** — these are
   correct as-is but coupled to whatever loop drives them. If
   we ever move to a proper monoio `select!` driver (the
   strategic fix for #1/#2), the 10 ms cadence on those becomes
   the floor latency for things like CMP status messages and
   WAL flushes. That's not a fix, it's a design constraint to
   carry into the rewrite.

---

## Appendix: oracle citation

Codex (gpt-5.4) was asked about the trade-off between
`monoio::task::yield_now()`, busy-spin, and full monoio-native UDP.
Verbatim conclusion:

> `monoio::time` is built on a hashed timing wheel whose level-0
> slot is 1 ms, so a `sleep(100 us)` is fundamentally the wrong
> tool for a microsecond-scale yield and can round up badly.
> Replacing that sleep with a scheduler yield is the only option
> in your list that can remove the timer-wheel tax without
> changing the socket model.

Action: cite when proposing the yield_now patch for #1/#2; do not
ship without measurement.
