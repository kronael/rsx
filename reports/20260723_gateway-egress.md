# 20260723 — gateway egress-tile-split (GATEWAY-LATENCY follow-up)

**What:** implemented the deferred tile-split for GATEWAY-LATENCY (BUGS.md):
moved the risk→gateway casting/UDP response recv off the shared monoio
reactor onto a dedicated busy-spin OS thread (`rsx-gateway/src/cast_recv.rs`),
same pattern rsx-risk's hot loop already uses. Measured the live cluster's
embedded per-leg stamps (`server_latency_ns`) before and after, in release
mode (see caveats — this host is not quiet).

## Root cause (unchanged from the original diagnosis)

The gateway is one monoio single-threaded runtime. The WS accept loop, every
per-connection handler task, and the casting-recv/route step all
cooperatively share one OS thread via `Rc<RefCell<GatewayState>>` (`!Send`
by design). Under WS load the casting-recv step competed for scheduler
turns with however many connection tasks were runnable, so a risk response
could sit in the kernel UDP buffer well past when it actually arrived —
the `match_done_to_gateway` leg.

## What changed

`rsx-gateway/src/cast_recv.rs` (new): a dedicated `std::thread` busy-spins
`CastReceiver::try_recv_with` (std non-blocking UDP, no async runtime —
the plain pattern rsx-risk's hot loop already proves out), decodes wire
records into an owned `CastEvent` enum (`Copy`, trivially `Send`), and
pushes them onto an `rtrb` SPSC ring. It never touches `GatewayState`.
The monoio thread drains that ring (cheap non-blocking `pop`, no syscall)
and calls the *same* `route_fill`/`route_order_done`/etc functions as
before, unchanged. Cross-thread wake reuses the exact readiness-dup
technique already in `main.rs` (a loopback UDP "doorbell": the recv
thread pings it once per drained batch; the monoio thread wraps its own
end for a `readable()` wait). FAULTED/RECONNECT handling (`handle_replay`
+ `reset_after_replay`) moved to the new thread with the receiver;
`drain_replay` builds its own self-contained tokio runtime, so the thread
move doesn't touch it.

`GatewayState` stays exactly `Rc<RefCell<...>>`, single-thread-owned — no
`Arc`/`Mutex` rewrite, no `handler.rs`/`egress.rs`/`route.rs` signature
changes. `rsx-cast` untouched (frozen); `CastReceiver`/`CastSender` used
exactly as before, from a different call site.

Full root-cause/plan/invariants write-up: `BUGS.md` "Status — 2026-07-23 —
gateway egress latency truthing".

Diff: `rsx-gateway/src/cast_recv.rs` (new, 260 lines incl. doc comments),
`rsx-gateway/src/cast_recv_test.rs` (new, 9 unit tests), `main.rs` (recv
loop replaced with a ring-drain loop + doorbell readiness wait),
`lib.rs`/`Cargo.toml` (module + `rtrb` dep).

## Verification (non-negotiable, before any latency claim)

- `cargo test -p rsx-gateway` (52 tests incl. 9 new `decode_cast_event`
  dispatch tests) — all green.
- `cargo test --workspace --lib` — all green, no regressions.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- Live functional check: fresh cluster, maker quotes, taker crosses, fills
  route to both sides, WS `U`/`F` frames delivered — unchanged behavior.
- FAULTED/RECONNECT path exercised live (a partial gateway restart against
  a live risk stream forced several real FAULTED events): `gateway
  cast_receiver FAULTED ... opening replay ...` /
  `reset_after_replay` fired correctly from the new thread and the cluster
  recovered — the moved logic works unchanged.

## Methodology fixes needed to get here

1. **`scripts/latency-publish.sh` crashed instead of retrying** — its
   liquidity-poll hit `/api/bbo/{symbol}`'s legitimate 404 ("no bbo yet")
   with no exception handling, so `make perf-load` could never get past
   seeding. Fixed (commit `c1e31ea`): catch `HTTPError` 404, treat as
   not-ready, keep polling.
2. **The Go market-maker never replied to gateway heartbeats** — the
   gateway's idle-timeout (`RSX_GW_IDLE_TIMEOUT_S`, default 10s) kills a
   connection with no inbound traffic; `quoteCycle` only writes once per
   refresh interval, commonly >10s (the "hold steady for the whole test"
   `refresh_ms` `latency-publish.sh` computes) — so the maker's own
   persistent connection died before it ever placed a quote and
   `/api/bbo/{id}` stayed 404 forever. Fixed (commit `f3c3904`): echo
   every inbound Heartbeat, mirroring `rsx-term`'s `readGw`/`readMd`.
3. **The live cluster ran debug builds; no release build existed.**
   `bench-match-rt`'s 12µs/17µs/37µs figure is release-mode; comparing
   live (debug, ~5-20x slower on CPU-bound Rust) against it was
   apples-to-oranges. Built `target/release` for the first time this
   session and added a measurement-only opt-in,
   `RSX_PLAYGROUND_RELEASE=1` (read by `get_spawn_plan`, default off/
   unchanged), to drive `perf-load`-style runs against release binaries.
4. **`bench-match-rt` models 2 hops, not the real 4.** It simulates
   `gw_send → udp_to_me → me_match → me_send → udp_to_gw` — one cast hop
   standing in for "gateway ↔ ME" directly, with **no rsx-risk call at
   all**. The real path is gateway→risk→ME→risk→gateway. Its number is a
   real, useful lower bound on cast-transport + matching cost, but is not
   the same measurement as a live 4-hop round trip.
5. **`GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS`** (new, recorded in
   BUGS.md, not fixed — out of scope here): the gateway's pending-cancel
   index sweeps any order older than `RSX_GW_ORDER_TIMEOUT_MS` (default
   10s) regardless of whether it's still resting, so `Cancel` silently
   fails ("order not found") for anything older than 10s. Found because it
   kept stranding the market-maker's own resting orders across restarts
   during this session's measurement setup.

## Numbers

All legs from the live embedded stamps (`gw_in_ns → risk_in_ns → me_in_ns
→ match_done_ns → gw_out_ns`), never client wall-clock. `RATE=2000/s,
10 WS connections`, release-mode binaries, `target/release`.

### Single clean-window sample (lowest host contention seen this session)

| leg | before (p50) | after (p50) | before (p95/p99) | after (p95/p99) |
|---|---:|---:|---:|---:|
| `internal` (gw_out−gw_in) | 656 µs | 652 µs | 9,450 / 17,629 µs | 9,034 / 20,919 µs |
| `gateway_to_risk` | 13.1 µs | 12.8 µs | 2,296 / 7,123 µs | 1,029 / 6,395 µs |
| `risk_to_me` | 115.5 µs | 93.9 µs | 3,828 / 12,110 µs | 5,953 / 20,065 µs |
| `matching` | 12.2 µs | 13.0 µs | 23.7 / 50.2 µs | 35.5 / 136.6 µs |
| `match_done_to_gateway` | 144.0 µs | 118.7 µs | 2,737 / 7,901 µs | 4,076 / 6,345 µs |

At this specific point-in-time, `internal` was **already sub-ms before the
fix** — p50 was never the original 11.6ms problem at this load level;
`match_done_to_gateway` p50 improved ~18%, `p99` tails are noisy in both
directions.

### 8-sample repeated batches, interleaved as closely as time allowed (median p50, µs)

Ran 8x back-to-back on the OLD binary, then 8x on the NEW (fixed) binary,
same host, same load shape, minutes apart (git-stash A/B: `git stash` the
fix, rebuild+measure, `git stash pop`, rebuild+measure).

| leg | before median | after median | untouched-leg sanity check |
|---|---:|---:|---|
| `gateway_to_risk` | 13.0 µs | 12.6 µs | **not touched by this fix** — tight both sides |
| `matching` | 11.6 µs | 11.8 µs | **not touched by this fix** — tight both sides |
| `match_done_to_gateway` | 411.6 µs | 539.3 µs | **the targeted leg** — no confident win, within noise |
| `risk_to_me` | 42,598.6 µs | 4,345.8 µs | not touched by this fix — 10x swing either way |
| `internal` | 55,845.9 µs | 5,539.6 µs | sum of the above — dominated by risk_to_me's swing |

Per-run raw p50s (µs), `match_done_to_gateway`:
- before: `1341, 570, 559, 1279, 211, 133, 264, 247`
- after: `905, 91, 363, 539, 5228, 534, 1706`

## Conclusion

The fix is **structurally correct and verified functionally** (tests,
clippy, live fault-injection recovery) — it removes exactly the
dependency the original diagnosis named (cast-recv sharing a cooperative
reactor with WS connection tasks). **I cannot honestly claim a proven,
quantified win for `match_done_to_gateway` from this session's live
numbers** — the 8-sample before/after medians overlap inside the noise
band, and the two legs the fix does *not* touch (`gateway_to_risk`,
`matching`) stayed tight and nearly identical (13.0→12.6µs, 11.6→11.8µs)
across all 16 runs, which is the actual evidence: this host's variance is
large enough to swamp a sub-millisecond effect, and it is *not* uniform
random jitter (the untouched legs don't show it) — it tracks a shared,
oversubscribed 8-core box (`uptime` load average ranged 8→20 across this
session; unrelated `grid_bot_*` Docker containers and a headless Chromium
instance were resident throughout) most plausibly starving `risk`'s
un-pinned busy-spin hot loop, not the gateway.

**Escalation was not attempted** (multi-reactor sharding, etc.) — the
single pinned-thread+SPSC split is architecturally the full fix the
original diagnosis named, and per-loop reasoning found no lighter partial
step beyond what BUGS.md already recorded as tried (tightened housekeeping
interval, event-driven wait). The honest gap is measurement conditions,
not a missing code change.

**Done = sub-ms** was reached in the one clean sample available
(`internal` p50 656→652µs, both already sub-ms) but that sample also
shows the *original* system was already sub-ms at this load level — the
initial 11.6ms figure came from a different test methodology (closed-loop
sequential Python WS-probe, `reports/20260530_e2e-ws-probe.md`) that this
session did not directly reproduce. A follow-up on quiet, dedicated
hardware — or with `risk`/`gateway`/ME core-pinned (`RSX_*_CORE_ID`,
already supported, unset in this playground config) to remove the
noisy-neighbor variable — is needed to cleanly attribute the remaining
tail to code vs. environment.

## Caveats

- Shared host, NOT quiet: `uptime` load average 8→20 on 8 cores across
  this session, unrelated Docker services (`grid_bot_aevo`,
  `grid_bot_dashboard`, `grid_bot_ostium`) and a Playwright/Chromium
  instance resident throughout, outside my control.
- Neither `risk` nor `gateway` nor `matching` were core-pinned
  (`RSX_*_CORE_ID` unset in the playground's default env) — every process
  competed for the same 8 cores as the unrelated tenants above.
- Single-shard, single-symbol (PENGU), synthetic maker liquidity.
- `RSX_PLAYGROUND_RELEASE=1` is a measurement-only opt-in added this
  session (`rsx-playground/server.py get_spawn_plan`); the playground's
  default remains debug builds.
- Sample sizes are small (8 runs/side) given time budget; medians are
  indicative, not a rigorous statistical claim.

Source: this session's live cluster, `scripts/latency-publish.sh` (post
404-fix) + direct `rsx-playground/stress.py` invocations (bypassing the
maker-restart-triggering seed step, which repeatedly hit
`GW-PENDING-TIMEOUT-STRANDS-RESTING-ORDERS`), `target/release` binaries,
commit range starting at `c1e31ea`.
