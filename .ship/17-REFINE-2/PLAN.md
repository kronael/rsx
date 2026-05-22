# 17-REFINE-2 — Plan

Execute the 6-item forced-rank refine list from
`.ship/16-CTO-CEO-REVIEW/SYNTHESIS.md` plus the 3 latency
probes deferred from the earlier (a)/(b) decision. The
review verdict was unanimous-NO; these changes are the
shortest path to a defensible v0.3.

Methodology context: `meta/cto-ceo-review.md`. Per the
methodology this is "act later" work — now we act.

## Scope: 9 items, 4 disjoint buckets

### Bucket 1 — Correctness (Rust, no playground)

- **F1.1** Silent fill drop on risk: `rsx-risk/src/main.rs:601`
  `let _ = fill_prod.push(...)` and 4+ peer sites (`571,
  758, shard.rs:1059, 1088, 1093`). Replace with
  stall-on-full + WARN counter (same pattern as
  `push_persist` at `shard.rs:185-192`).
- **F1.2** Workspace `let _ =` sweep: CTO claimed 18 sites
  remain despite MEMORY.md's "0 violations". Find and fix
  or annotate every one.
- **F1.3** Matching engine event drop: `rsx-book/src/book.rs:88-94`
  drops events when `event_len >= MAX_EVENTS = 10_000`,
  contradicting invariant "ME never drops events". Decide:
  raise MAX_EVENTS, or apply backpressure to caller.

### Bucket 2 — Security wire (rsx-auth + rsx-gateway)

- **F2.1** `rsx-auth/src/rsx_auth/jwt_util.py:14-23` must
  emit a `jti` claim on every minted token
  (`uuid.uuid4().hex`).
- **F2.2** `rsx-gateway/src/jwt.rs:107-110` `JtiTracker::record`
  currently passes when `jti` is `None`. Tighten: missing
  jti from a non-test token = reject with 401. Add a counter.
- **F2.3** Integration test: mint a real token via
  `rsx-auth` HTTP, replay it twice through ws_handshake,
  assert second handshake gets 401 `jti replay`.

### Bucket 3 — Playground (server.py + pages.py)

- **F3.1** i64 tick-size formatter on `/risk`, `/topology`,
  `/maker`. Acceptance: `COLLATERAL 999999972019150` →
  `$1,000,000.00`; bbo `bid=49900 ask=50100` →
  `0.0499 / 0.0501 / spread 0.0002`. Operator-debug
  surfaces (`/wal` timeline) keep raw with header column.
- **F3.2** Polling thundering herd. `/x/health` 75 s,
  `/x/key-metrics` 15 s under self-poll. Acceptance:
  cold-open of `/overview` paints all panels within 1 s,
  `curl /x/health` returns < 200 ms. Likely a per-endpoint
  TTL cache + skip the `read_logs` scan on the health path.
- **F3.3** Failing-invariant → red health wire. Today
  `/verify` FAIL row + `/x/health` YELLOW=70. Acceptance:
  any `verify_results` entry with `status="fail"` drops
  health to RED ≤ 49 within one poll.
- **F3.4** Kill the `1117 / 1117 / 1117` CMP-counter ghost.
  Wire `/x/cmp-flows` to per-pipe WAL-stream-derived counts
  so the three pipes show distinct numbers.

### Bucket 4 — Latency probes + CI gate

- **F4.1** Native Rust client probe (new binary
  `rsx-cli bench-probe` or `rsx-bench-client` crate). Same
  WS handshake, same N/F flow, no Python. Records its own
  measurement; meant to isolate Rust-side latency from the
  Python aiohttp overhead.
- **F4.2** Gateway-only RTT probe in the playground (no
  ME involvement). Submits a request that the gateway can
  ack without entering risk/ME. Isolates Python aiohttp
  overhead alone.
- **F4.3** Per-stage instrumentation: `t_in_gateway`,
  `t_out_to_risk`, `t_in_back_from_risk`, `t_out_to_client`
  via `tracing::info!(target = "latency", ...)`. Dashboard
  parses + shows on `/latency`.
- **F4.4** CI bench gate. `make perf` reads
  `bench-baseline.json`, fails if `e2e_us.p50` regresses
  > 10 % from a sealed reference. `specs/2/22-perf-verification.md`
  already describes this — implement it.

## Dispatch

4 parallel general-purpose subagents (CLAUDE.md max). Each
gets one bucket. File ownership:

| Bucket | Owns | Won't touch |
|--------|------|-------------|
| 1 (Rust correctness) | rsx-risk/, rsx-book/, rsx-matching/, rsx-marketdata/ for `let _ =` sweep | rsx-playground/, rsx-auth/, rsx-gateway/ws.rs |
| 2 (Auth + jti) | rsx-auth/, rsx-gateway/src/jwt.rs, rsx-gateway/src/ws.rs, rsx-gateway/tests/jwt_ws_e2e_test.rs | everything else |
| 3 (Playground) | rsx-playground/server.py, pages.py, tests/ | rsx-gateway/, rsx-auth/, rsx-risk/, rsx-book/ |
| 4 (Probes + gate) | new rsx-cli bench-probe (or new crate), scripts/, Makefile, specs/2/22-perf-verification.md edits | rsx-risk/, rsx-book/, rsx-gateway/src/jwt.rs |

## Convergence

After all 4 land:
- `make lint` + `cargo test --workspace --lib --tests`
  green
- Run an updated `make latency-publish` against the
  cluster; capture new p50/p99 and append a note to
  `bench-baseline.json` showing pre/post numbers.
- Re-run the CEO browser audit against the same UI surfaces;
  one-page diff vs `CEO-REPORT.md` (does it still say NO?).
- Tag candidate v0.3.0-rc1.

## Out of scope

- Trade UI ("Loading..." forever) — separate sprint,
  blocked on rsx-webui WS reconnect logic that needs
  design work.
- The 234× over budget itself — that's a multi-quarter
  project (kernel-bypass / DPDK swap per spec §"Later"),
  not a refine pass deliverable.
- Publishing — repo policy (CLAUDE.md "Publishing").
