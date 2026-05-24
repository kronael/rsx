# FIX-REPORT — .ship/27 P0+P1 critique-fix pass

CTO 58/100 (hold) + CEO 14/100 (not yet) — fix-up sprint.
Master at sprint start: `5cca081`.

## Commits

```
56c7ec4 [playground] start-all: do not auto-start maker (overflows default rmem)
f24d005 [specs] delete 50-wedge.md (wedge framing dropped 2026-05-24)
622c54f [docs] rsx-cast README: WalWriter::new arity, poll->try_recv_with; JtiTracker wired
09d4d87 [cast] wal: prune segments older than RETENTION_NS (4h) on rotate
fdec826 [cast] clamp oldest_missing_run to REORDER_CAPACITY (LAN DoS fix)
ca941a7 [playground] timeout-as-accept -> HTTP 504 + amber UI
7b46152 [start] env: RSX_*_CMP_* -> RSX_*_CAST_* (matches code rename in 3da5d9d)
4a0c5ff [mark] install rustls aws-lc-rs CryptoProvider at startup
```

Tests: 878 → 879 passed, 0 failed, 46 ignored. `cargo check --workspace`
clean. No new clippy warnings.

## F-N1 diagnosis — the actual root cause

**Two compounding bugs, both in the spawn plan; the prompt was right that
the rename sprint broke it.**

### Root cause 1 — env-var rename never propagated to `start`

Commit `3da5d9d` (2026-05-24, "remove history/prior references from code
comments") renamed every `RSX_*_CMP_*` env var to `RSX_*_CAST_*` in the
matching/risk/gateway/marketdata/mark binaries. `start` (the Python
spawn-plan script) was NOT updated. Effects:

- `RSX_ME_CMP_ADDR` set by `start` → ignored by `rsx-matching` → ME falls
  back to its default `127.0.0.1:9100`.
- `RSX_ME_CMP_ADDRS` set by `start` → ignored by `rsx-risk` → Risk falls
  back to `{ sid=10 -> 127.0.0.1:9110 }` (its own default).
- ME-pengu (sid=10) bound `:9100`; Risk sent PENGU orders to `:9110`.
  **Port mismatch — Risk's order packets arrived at a port no one listened
  on.** WAL stayed 0 bytes. ME logs showed `accepted=0 cancelled=0
  last_seq=0`. CEO §F-N1 reproduced exactly.

The accidental coincidence that PENGU (sid=10) survives at all is because
Risk's default ME-map happens to assign sid=10 → port 9110, which still
collides with ME's bound port 9100 — so it's PENGU all the way down but
NOTHING was actually received.

Fixed in `7b46152` by renaming all `RSX_*_CMP_*` → `RSX_*_CAST_*` in
`start`. Verified with a Python repro that builds the spawn plan and prints
each effective env var — every address now uses `CAST` and matches what
the binaries read.

### Root cause 2 — maker overruns default UDP rmem

Once the env wiring was fixed, the FIRST IOC order made it gateway → risk →
me → published events — but the order's `risk_out` / `gateway_out` stages
never logged because the maker was generating ~40 orders/sec (5 levels ×
2 sides × 500ms refresh) and the resulting UDP packet stream exceeded
`/proc/sys/net/core/rmem_max` (208 KB on stock kernels). Risk's
`me_receiver` hit a gap → FAULTED → panicked (FAULTED recovery requires
a DXS replay server on Risk, which Risk doesn't run — a known POC-grade
limitation documented in `README.md` "What's not done" and CTO §critical
#9). Supervisor restarted Risk, but ME's seq counter was already past the
new receiver's first-sync point, so it FAULTED again, etc.

Fixed in `56c7ec4`: `start_all()` no longer auto-spawns the maker.
Operators start it manually from `/controls` when they want depth. The
maker isn't deleted, just made opt-in.

### Trace proof of the fix

Submitted `{symbol_id=10, side=buy, price=0.07, qty=100, tif=IOC}` against
a clean book pre-seeded with a GTC sell at 0.06. Per-stage latency lines:

```
gateway_in              t_us=0
gateway_cmp_recv        t_us=1065
gateway_route_serialize_done  t_us=1072
gateway_out             t_us=1072
risk_in                 t_us=27
risk_out                t_us=148
risk_cmp_send_done      t_us=159
me_in                   t_us=106
me_dedup_done           t_us=116
me_wal_accepted_done    t_us=121
me_match_done           t_us=129
me_wal_events_done      t_us=164
me_index_done           t_us=166
me_out                  t_us=166
```

(Per-process anchors; not summable across processes.)

WAL after:
```
seq=1 ORDER_ACCEPTED (sell 0.06)
seq=2 ORDER_INSERTED (sell 0.06)
seq=3 ORDER_ACCEPTED (buy IOC 0.07)
seq=4 FILL (price 0.06, qty 100) — taker oid 019e5c65...340d
seq=5 ORDER_DONE (maker, filled, remaining=0, status=FILLED)
seq=6 ORDER_DONE (taker, filled, remaining=0, status=FILLED)
```

HTTP response: `order pg65298ec1044e5 accepted (12676us)`. The 12.7ms is
WS-connect + JWT validate + the actual round-trip.

## F-N2 fix location

`rsx-playground/server.py`:
- Line 4425 (api_orders_test): `timeout waiting for response` → return
  HTTPResponse `status_code=504` + amber "order {cid} timeout: no
  matching-engine response in 2s"
- Line 5138 (quick-submit form): same pattern, amber "timeout (cid)".

The previous `order["status"] = "accepted"` lie is gone; the recent_orders
list now stores `status=timeout`, which falls out of the /verify
"exactly-one completion" check.

Note on GTC ambiguity: spec 49-webproto.md §54 explicitly says "no
accepted ACK" for resting GTC. A 2s timeout could legitimately mean
"order is resting." We surface this honestly as amber/timeout rather
than pretending it's an unambiguous accept; the operator can verify
state via `/v1/orders` or the WAL.

## F-N3 fix location

`rsx-mark/Cargo.toml`: added direct `rustls = { version = "0.23",
features = ["aws-lc-rs"] }` dep.

`rsx-mark/src/main.rs` (after `tracing_subscriber::fmt::init()`):

```rust
let _ = rustls::crypto::aws_lc_rs::default_provider()
    .install_default();
```

Why this happened: rsx-cast pulls `rustls` with `aws-lc-rs`, and
tokio-tungstenite's `rustls-tls-native-roots` feature pulls it with
`ring`. With both features enabled, rustls 0.23 cannot auto-select a
provider and the first TLS handshake panics. Installing aws-lc-rs as the
default at startup makes this deterministic.

Checked the other crates: `rsx-gateway` and other binaries don't use TLS
directly (gateway WS is `ws://`, no rustls path). `rsx-cast` already
installs aws-lc-rs in its test suite (`replication_server_test.rs:41`)
and in its compare bench (`benches/compare_all.rs:308`). No other binary
needed the fix.

## CTO #5 mitigation — oldest_missing_run clamp

`rsx-cast/src/cast.rs:1100-1120`. Replaced the unbounded
`while seq <= self.highest_seen` walk with:

```rust
let upper = self.highest_seen
    .min(from.saturating_add(REORDER_CAPACITY as u64));
while seq <= upper { ... }
```

REORDER_CAPACITY = 2048. A spoofed heartbeat with `highest_seq` near
u64::MAX can no longer wedge the receiver thread on the NAK path. The
clamp value is also the in-flight gap window the receiver tolerates
before transitioning to FAULTED, so anything beyond it is the wrong
recovery path anyway (DXS replay handles it).

Existing 82 cast tests still pass; no new test added because the failure
mode is "no infinite loop" which is hard to assert positively in a unit
test — the loop change is straightforward static reasoning.

## WAL retention decision + justification

**Picked (a) — implement it.** Reasoning:

- 5+ surfaces (CLAUDE.md, FEATURES.md, BLOG.md, PROGRESS.md,
  rsx-cast/ARCHITECTURE.md) promise 4h hot retention. Dropping the claim
  in five places without enforcing anywhere is a bigger surface area than
  the implementation.
- The implementation is ~30 LOC: a single `prune_old_segments(wal_dir,
  stream_id, retention_ns)` function called at the end of `rotate()`.
  Iterates `read_dir`, skips the active file, parses segment filenames,
  checks `metadata().modified()` age vs `RETENTION_NS`, unlinks. Errors
  are logged + skipped (never propagated — pruning failure must not
  break the writer).
- `RETENTION_NS = 4 * 60 * 60 * 1_000_000_000` as a const at the top of
  `wal.rs`. Matches the spec.
- ARCHIVE / recorder tier handles long-term durability unchanged.

Constraint from the prompt: "small enough to be implementable in one
commit (file-age check during rotation)". Implemented exactly that
shape — no async timer, no separate GC thread, no policy knob; pruning
happens during rotation, which is when a writer is already touching the
directory.

Test added: `rotation_prunes_segments_older_than_retention` (rsx-cast
wal_test.rs). Writes 30 records → triggers rotation; backdates the
resulting segment files by 5h using `File::set_modified`; writes 30 more
records → triggers another rotation; asserts the backdated segments are
gone and the active file survives. Test passes (`cargo test -p rsx-cast
--lib rotation_prunes`).

## README diff for #2/#3/#4

`rsx-cast/README.md`:

- Quick-start sender: `WalWriter::new(stream_id, &wal_dir, /*
  max_file_size */ 64 * 1024 * 1024)?` — 3-arg signature matching the
  real one. Added comment naming the new 4h-retention behaviour.
- Quick-start receiver: `CastReceiver::new(bind_addr, sender_addr)?` —
  2-arg signature (no stream_id). Replaced `rx.tick()` (dead since R1)
  and `poll`-named delivery with `try_recv_with`. Pattern A "see below"
  reference still valid.
- "What it gives you" zero-heap claim: qualified to send-path only, with
  explicit "the receive path's `try_recv_with` callback delivers
  &[u8] from the receiver buffer; the convenience `try_recv` (owned
  Vec) does allocate one Vec per packet" — matches CTO claim 2 verdict.
- Trailing `try_recv` shim note: kept, reworded for clarity.

`README.md` (root):
- "JWT replay protection" bullet: corrected to "wired into the WS
  handshake (rsx-gateway/src/ws.rs)" with the FIFO eviction caveat
  carried over.

`rsx-gateway/README.md`:
- "Auth" bullet on JtiTracker: corrected from "dormant — not yet wired
  through ws_handshake" to "wired into ws_handshake (src/ws.rs)" with
  cap noted.

## Re-demo evidence

Fresh start sequence:

```
./playground start                                    # server up
curl -X POST /api/processes/all/start?confirm=yes     # 6 procs running
# All 6: gw-0, mark, marketdata, me-pengu, recorder, risk-0
# Mark log: 'ws connected to wss://stream.binance.com:9443/...'  (no panic)
```

Submit a real IOC:

```
curl -X POST /api/orders/test?confirm=yes \
  -d 'symbol_id=10&side=buy&price=0.07&qty=100&tif=IOC'
→ HTTP 200 'order pg65298ec1044e5 accepted (12676us)'
```

WAL state after:

```
$ rsx-cli dump tmp/wal/pengu/10/10_active.wal
seq=1 ORDER_ACCEPTED  (the pre-seed GTC sell at 0.06)
seq=2 ORDER_INSERTED
seq=3 ORDER_ACCEPTED  (the IOC buy at 0.07)
seq=4 FILL            (price 0.06, qty 100, taker=buy)
seq=5 ORDER_DONE      (maker, status=FILLED, filled=100, rem=0)
seq=6 ORDER_DONE      (taker, status=FILLED, filled=100, rem=0)
total: 6 records
```

WAL grew, full lifecycle present, exactly-one ORDER_DONE per order,
fills precede ORDER_DONE — all four canonical invariants from the
matching spec hold.

Per-stage latency lines (gateway → risk → me → ... → gateway): see
"Trace proof" above.

The demo trades.

## Deferred items + justification

Each item below is acknowledged but NOT shipped in this sprint, per the
prompt's scope ("don't add features").

- **F-N4 — synthetic-book `/x/book` from maker config.** Real fix is a
  badge on the rendered table when `_book_snap` is empty. Cosmetic; not
  blocking. Defer.
- **F-N5 — `/api/latency-probe-gw` `ok=true` while body has 1007.**
  Truthiness bug; the fix is a single `ok = error_code == 0` change but
  it needs to ship together with a sensible default-symbol picker. Not
  blocking the demo. Defer.
- **F-N6 — stress run `submitted=0 -> PASS`.** Same family as F-N2;
  needs the harness's pass criterion to require `submitted > 0`. Defer.
- **F-N7, F-N8 — `/verify` mixes archive + live and PASSes 0-byte WALs.**
  Defer; fix in the same sprint that audits the rest of `/verify`.
- **F-N9, F-N10 — `/x/order-trace` doesn't read `tif`; renders IOC as
  resting.** Renderer bug; defer.
- **F-N11 — gateway CMP rebind AddrInUse on restart.** Per the prompt:
  SO_REUSEPORT was dropped intentionally this session (single-owner port
  discipline). The fix is supervisor-side: wait for the prior PID's UDP
  socket to release before respawn. `_process_watcher` currently calls
  `spawn_process` directly without that wait. Defer — needs a separate
  PR with a `wait_for_port_free` helper and the watcher rewire.
- **F-N12 — gateway max-conn-per-user cap.** Playground opens a fresh
  WS per submit (CTO §F-N12). The fix is either a per-user pool in
  `send_order_to_gateway` or lifting the cap for `user_id=1` in dev.
  Defer.
- **F-N13 — `/api/processes` restart counter is 0.** UI counter wiring
  bug. Defer.
- **F-N14, F-N15, F-N16 — walkthrough crate/test count, CDN Tailwind on
  /docs, pulse pill 5/10 mismatch.** Defer; small UI/copy fixes that
  can ship together.
- **CTO #6 — consumers use allocating `try_recv` while CLAUDE.md asserts
  "zero heap on hot path" unqualified.** The send-path qualification is
  now in the rsx-cast README; root CLAUDE.md still asserts the broader
  claim. Defer — needs either a wider audit of every consumer or a
  CLAUDE.md edit that names the qualifier.
- **CTO #7 — `rsx-risk/src/shard.rs:843-846` allocates per BBO.** Known
  R-N5 leftover. Defer.
- **CTO #9 — risk/marketdata/gateway panic on `CastRecv::Faulted`.**
  Acknowledged POC-grade in `README.md` "What's not done". Today's
  fix (no auto-maker) avoids the worst trigger; the proper fix is
  wiring DXS replay through risk + marketdata + gateway. Multi-week
  scope. Defer.
- **CTO #10 — BBO record CRC'd twice.** Defensible carve-out per CTO's
  own analysis. Defer (probably document, not fix).

## Recommendation for next sprint

Three buckets, by ROI:

1. **Wire FAULTED recovery for risk + marketdata + gateway** (CTO #9 /
   F-N1 root cause #2). This is the only thing standing between "demo
   trades a few orders" and "demo trades at maker-realistic rates". One
   developer × one week.
2. **Playground honesty pass** (F-N4–F-N10 + F-N13–F-N16). Cluster the
   small UI lies into one sprint so the dashboard stops over-claiming.
   One developer × three days.
3. **Sysctl + bench harness** for UDP rmem on demo hosts: ship a `make
   tune-host` target that bumps `/proc/sys/net/core/rmem_max` to
   8 MB so the maker doesn't have to be opt-in for the demo. Half a day.
