# CEO Audit — RSX (.ship/27, 2026-05-24)

Reviewer persona: founder. Tools: live browser + curl + headless
Playwright against `http://localhost:49171`. Postgres bootstrapped via
docker (host port 5432). Master at audit start: `5cca081`. Round-2 CEO
report at `.ship/20-CTO-CEO-REVIEW-2/CEO-REPORT.md` was scored 22/100.

## 0. Setup

- Stale playground (process pid 1809687) was already listening on
  `:49171` from a session that started 10:12 the same day — 7 hours of
  uptime, 230 min CPU. Killed it before starting fresh. Note for the
  team: `./playground start` could detect "stale own process" and
  offer a clean restart instead of erroring on port-in-use.
- `cargo build` was not needed — debug binaries already present.
- `start-all minimal` returned `error: 400` until I added
  `?confirm=yes`. Confirmation header is documented in the API doc
  but the CLI doesn't pass it.
- Cold-start time from "Start All" click to processes-reported-as-
  running: **22s**. To "Submit Order being responsive": effectively
  **never** in this session — see §2.

## 1. Did "Start All" actually start every RSX process?

**No.** 6 of 7 processes started; **`mark` panicked 5 times and was
abandoned by the supervisor.** Health pill at fresh start: 35/red.

The mark panic is the rustls `CryptoProvider` bug:

```
fatal: panicked at .../rustls-0.23.36/src/crypto/mod.rs:249:14:
Could not automatically determine the process-level CryptoProvider
from Rustls crate features.
```

Source: the binary doesn't call `CryptoProvider::install_default()`
before opening `wss://stream.binance.com:9443/...`. File:
`rsx-mark/src/source.rs` (WS client). After supervisor backoff
exhausts, mark sits in `state=blocked, restarts=5/6` forever — the
playground UI shows `mark — blocked` and the health pill stays red.

Manual `/api/processes/mark/restart?confirm=yes` **does** bring it
back, instantly, with no code change. So the bug is racy
(connect-faster-than-init?), but the supervisor's "5-strike → quit"
policy is wrong: each strike happens within ~8s of the prior, the
backoff is too short, and there is no UI affordance saying "mark has
been blocked since boot; try Restart". A first-time visitor sees
"35/red, panic in logs, mark blocked" within 10 seconds of clicking
Start All — that is a hostile first impression.

## 2. Demo flow

Sequence: submit order → watch fill → verify WAL → verify
marketdata.

### Submit order

Endpoint `/api/orders/test?confirm=yes` with valid PENGU buy IOC.

Every call returns:
```
<span class="text-emerald-400 text-xs">order pg65...accepted</span>
HTTP 200 in 2.003 ± 0.01 s
```

Five consecutive sequential orders, **every single one** takes 2.00s.
20 concurrent orders queue serially, fastest 2.21s, slowest 8.59s.

That 2-second-flat is not real fill latency — see `server.py:4424`:

```python
if err == "timeout waiting for response":
    order["status"] = "accepted"            # <— LIE
    return HTMLResponse(f'order {cid} accepted')
```

`send_order_to_gateway` waits 2s for a reply, never gets one (the
gateway/ME chain is broken — see below), then the playground
**displays the timeout as success**. The user sees green
"accepted" forever and has no way to know it never matched.

### Watch the fill

`/x/live-fills?sym=PENGU` → `no fills yet`. Forever.

`POST /api/latency-probe?confirm=yes` →
`{"ok":false,"error":"timeout waiting for fill"}` consistently. This
endpoint was a CEO round-2 highlight (returned 15 965 µs E2E). It
now returns timeout 100% of the time.

### Verify WAL

`rsx-cli dump tmp/wal/pengu/10/10_active.wal` → **`total: 0
records`**. The active WAL file is **0 bytes** and `wal_seq.txt`
contains **`0`**, despite:

- maker reporting 2 834 orders placed, 2 819 cancels sent over 16
  minutes (`tmp/maker-status.json`),
- 686+ `risk_in` entries in `log/risk-0.log`,
- gateway logging hundreds of `gateway_in` events with `t_us=0`.

The matching engine receives nothing. ME log only contains its
startup banner; no order has ever been accepted in this session:

```
rsx_matching::wal_integration: wal replay: accepted=0 cancelled=0
                              last_seq=0
```

### Verify marketdata

Direct WS subscribe to `ws://localhost:8180` for PENGU L2/T/BBO:

```
subscribed PENGU
TIMEOUT after 0 msgs (8s)
```

Zero market data. Yet `/x/book?symbol_id=10` shows a clean depth
ladder (5 levels each side, tight spread). That's because **the
playground falls back to `_maker_book(symbol_id)` which synthesises a
ladder from `maker-status.json` mid_prices** (`server.py:6967`). The
book on the dashboard is a fiction painted from the maker's *desired*
mid; nothing in the actual exchange is matching.

The Trade UI (`/trade/`) loads (the round-2 bundle/server schema
mismatch is fixed — bundle now reads `.symbols`), but every header
field reads `--`: Bid, Ask, Mark, Index, 24h Change/High/Low/Vol,
Funding. The MD WS is "connected" (the dot is green) but no frames
ever arrive. Switching from BTC → PENGU in the dropdown does not
help — both show `--` because neither symbol has an active ME flow.

### Cross-process counters

`/x/cmp-flows` shows `Gateway→Risk 0/0/0/0`, `Risk→ME 0/0/0/0`,
`ME→Mktdata 0/0/0/0` after 16 minutes of maker traffic. Round-2 CEO
called this "quiet lying"; in round-3 the counters are still 0/0/0/0
and the system underneath is now also producing 0 throughput, so the
quiet-lying counters happen to be true. That's not a fix — it's a
worse-of-two-worlds coincidence.

## 3. Injected faults (5)

### 3.1 Kill ME mid-flow

```
pkill -9 -f rsx-matching
```

Supervisor restarts within 5 s with a fresh PID. **Restart counter on
`/api/processes` stays at `restarts=0`.** Snapshot replay logs
`accepted=0` (nothing to replay). Health pill rises back to amber
when the new ME registers. Acceptable mechanically; the *visible*
recovery story is missing — the restart count never increments, and
the dashboard never shows "ME just restarted, replaying from seq N".

### 3.2 Kill marketdata

```
curl -X POST /api/processes/marketdata/kill?confirm=yes
```

Returns `marketdata killed`. **Marketdata stays `stopped` indefinitely
— no auto-restart.** Compare with the matching engine and gateway
which DO auto-restart on `pkill -9`. So the supervisor's policy
differs by process and method (`pkill -9` is restarted; UI `kill`
endpoint is not). Manual `/api/processes/marketdata/restart` works
first try.

### 3.3 Kill gateway and submit order

```
pkill -9 -f rsx-gateway
curl -X POST /api/orders/test?confirm=yes ...
→ HTTP 200 "order pg6529865ad0b02 accepted" in 2.004 s
```

Gateway was dead when the order was submitted. The endpoint checks
`{"gw-0"} in scan_processes() running set`, which lags by up to 2 s
(the cache window), so it considers gateway "running" while it's
actually mid-restart. Then the order times out and the
`timeout-as-accept` code path lies green. Round-2 CEO had **ghost
submissions** as a critical; round-3 they are still critical.

Worse: gateway sometimes fails to restart cleanly. Log shows:
```
failed to bind CMP receiver: Os { code: 98, kind: AddrInUse }
```
The previous gateway's socket hadn't been released. The supervisor
must have eventually retried successfully, because gateway came up,
but a customer would have seen `502/connection refused` during the
window.

### 3.4 Malformed orders

| Input                            | Response                              |
|----------------------------------|---------------------------------------|
| empty body                       | `price not tick aligned` (default 0)  |
| `symbol_id=999`                  | `unknown symbol` ✓                    |
| `price=-1`                       | `price not tick aligned` (passes!?)   |
| `qty=10000000000` ($500M notional) | `accepted` (timeout-as-accept)      |

Negative price `-1` is divisible by tick `1` so the divisibility
check passes; there is no separate `price > 0` check. The 10-billion-
PENGU order is "accepted" with no margin warning because the entire
risk loop is silent (see §2).

### 3.5 Burst (20 concurrent)

```
for i in $(seq 1 20); do (curl ... &) done
```

Wall: 8.59 s. Median per-order: 4.4 s. Gateway log shows:
```
rejected connection: max connections per user
```
× 11 lines. The per-user connection cap fires at ~5 concurrent
submissions from the same user_id, and the playground does no
connection-pool reuse — each submission opens a fresh WS, so 20
concurrent → 15 rejected at the gateway. Not visible in the HTTP
response (still says "accepted" via timeout path). For a "100 rapid
orders" demo this would emit 75-95% silent failures.

## 4. NEW findings (≥5 required; 16 below)

NEW = not in `.ship/15-PLAYGROUND-AUDIT/F1-F28` and not in
`.ship/20-CTO-CEO-REVIEW-2/CEO-REPORT.md`'s round-2 critical list
(R1-R5) or "negative surprises" list.

### F-N1 CRITICAL — matching engine receives no orders for the entire session

After 16 minutes with maker active + ad-hoc orders + a stress run +
a latency probe: **WAL `10_active.wal` is 0 bytes, `wal_seq.txt`
contains `0`, ME log shows `accepted=0 cancelled=0 last_seq=0`,
direct MD WS subscribe returns nothing.** The demo flow does not
function end-to-end.

- Repro: any `./playground start-all minimal` + `curl /api/orders/
  test?confirm=yes` with valid form data.
- Observed: order returns green "accepted" in 2.003 s.
- Expected: order matches against the maker's quotes; WAL grows;
  marketdata WS emits an `F` frame.
- Severity: **critical**. This is the only thing a counterparty
  cares about and it does not work.

### F-N2 CRITICAL — timeout-as-accept lies green

`rsx-playground/server.py:4424`. When `send_order_to_gateway` returns
`err == "timeout waiting for response"`, the playground stamps the
order `status=accepted` and renders a green pill. **All "accepted"
responses in this session are timeouts** — the system has zero true
acks for non-maker orders. The bug compounds F-N1: the user has no
signal that anything is wrong.

- Repro: any order while ME is dead or risk is rejecting silently.
- Expected: amber/red pill with "no response from matching engine"
  and HTTP 504 or 503.
- Severity: **critical**.

### F-N3 CRITICAL — `mark` panics on rustls CryptoProvider, supervisor gives up

`rsx-mark/src/source.rs` (binance WS connect path). Missing
`rustls::crypto::CryptoProvider::install_default(
rustls::crypto::aws_lc_rs::default_provider())` before the first
`wss://` connect. Each fresh process panics within ~1 s; supervisor
retries 5 times in 35 s; then marks `state=blocked` forever. Health
pill stuck at red until manual operator intervention.

- Repro: clean `./playground start-all` from cold state.
- Observed: `mark` blocked at boot, health pill 35/red.
- Severity: **critical** for a demo (the first thing a counterparty
  sees on the health pill is "RED, panic in logs"); **major** in
  isolation.

### F-N4 MAJOR — book ladder synthesised from maker config, not exchange state

`server.py:6967` `_maker_book()`. When no real `_book_snap` data is
present, `/x/book` returns a ladder computed from
`maker-status.json` `mid_prices`, `spread_bps`, `num_levels`,
`qty_per_level`. The fall-through is silent — there is no
"synthetic" / "from maker config" badge on the rendered table.

- Repro: kill `me-pengu`, observe `/x/book?symbol_id=10` still
  shows a clean ladder.
- Observed: 5-level depth ladder, sensible-looking spread, no
  warning.
- Expected: either "no book" or a "synthetic (maker config)" badge.
- Severity: **major**. A demo viewer is shown a ladder that has
  zero connection to anything the exchange did.

### F-N5 MAJOR — `/api/latency-probe-gw` reports `ok=true` while body has error_code 1007

```
{"ok":true,"elapsed_us":442,"error_code":1007,
 "error_msg":"unknown symbol"}
```

The probe uses a hardcoded symbol (likely `BTC`) without checking
that an ME for it is running. The success flag should be
`error_code == 0`. Investor sees "Latency probe OK 442 µs" and may
miss the error string.

- Source: `server.py:6407` (probe-gw handler).
- Severity: **major** (truthiness bug in a demo number).

### F-N6 MAJOR — stress run reports `submitted=0 accepted=0 → PASS`

`/api/stress/start?rate=10&duration=10` ran for 25 s, produced
`p99=0us < target=50000us: pass` in `log/stress.log`. The stress
client failed to submit a single order (presumably the same
gateway-response timeout bug), but the harness's pass criterion is
"p99 < target", and `0 < 50000` is trivially true. So the smoke test
passes because nothing happened. Compare round-2 CEO finding "stress
page reports no tests run during/after a stress run" — not fixed,
and now compounded by a false-PASS in the report file.

- Source: `tmp/stress/stress-*.json` + `scripts/stress.py` (?)
- Severity: **major**. CI gates that consume this `pass` label
  will green-light a fully-broken pipeline.

### F-N7 MAJOR — `/verify` mixes archived WAL state with this-session state

`/verify` reports "Exactly-one completion per order: **34 orders, no
duplicates** — PASS" while the current `wal_seq.txt` is `0` and
nothing has been written this session. The 34 comes from
`tmp/wal/archive/10/10_2026-05-24.wal` (residual from an earlier
session). Mixing live + archive in a single PASS row hides "the
current session is fully dead".

- Source: `rsx-playground/verify.py` (?) or `server.py` verify
  routes.
- Severity: **major**. Counterparty reading /verify is told
  "system passed 12 of 17 invariants" while the system is in fact
  not exchanging anything.

### F-N8 MAJOR — `/verify` PASSes "WAL stream btc: 1 files, 0.0B" while no me-btc exists

No `me-btc` exists in `minimal` scenario, the active WAL has 0 bytes,
yet the row is green PASS. Same for `sol`, etc. PASS criterion
should require **non-zero file size** (or at least require an ME
process to be running for that symbol).

- Severity: **major**. Repeated PASSes for "dead" streams dilute
  the page's signal value.

### F-N9 MAJOR — `/x/order-trace` reports IOC orders as "resting in book"

```
<oid: pg6529865ad0b02>
✓ submitted PENGU buy 10@0.060000 at 23:12:34
✓ routed     gateway accepted (-us)
⚠ open       resting in book
```

The order was `tif=IOC` so it cannot rest; either it filled (display
should say "filled") or it was rejected (display should say
"rejected"). The trace logic doesn't read the order's `tif` field
when picking the lifecycle stage. Also `(-us)` instead of "no
latency reported" is a UI bug — the dash literal is rendered, the
user can't tell if it's "unknown" or "0".

- Source: `rsx-playground/pages.py` (order trace renderer).
- Severity: **major**.

### F-N10 MAJOR — order trace shows $500M-notional 10-billion-PENGU IOC quietly "resting"

Same renderer as F-N9. Submitted `qty=10000000000 price=0.05` →
notional ≈ $500M, no risk pre-check rejection, displayed as `open —
resting in book`. The risk engine *would* reject this with
InsufficientMargin (user 1 has $0 collateral in the running DB), but
the dashboard never shows the rejection because the response never
flows back.

- Severity: **major** (compounds F-N2; counterparty sees a
  half-a-billion-dollar order quietly resting on the book).

### F-N11 MAJOR — gateway fails to bind CMP receiver after restart ("Address already in use")

```
failed to bind CMP receiver: Os { code: 98, kind: AddrInUse,
message: "Address already in use" }
```

After `pkill -9 rsx-gateway`, the next supervisor-launched gateway
process attempted to bind its CMP receive socket before the kernel
released it. Supervisor retried until success but the recovery
window — during which `/x/order` returns "accepted (timeout)" —
exceeded 5 s on my run. Add `SO_REUSEADDR` (or
`SO_REUSEPORT` if the kernel build supports it) to the gateway's
CMP socket creation.

- Source: `rsx-gateway/src/main.rs` CMP UDP bind site.
- Severity: **major**.

### F-N12 MAJOR — gateway "max connections per user" cap blocks dashboard burst tests

```
rejected connection: max connections per user
```

20 concurrent `curl /api/orders/test` from the playground (all using
`user_id=1`) → 15 rejected at the WS handshake. The playground
opens a fresh WS per submit instead of pooling. Either lift the cap
for the playground (it's a dev tool), document it on `/orders`, or
add WS-pool reuse in `send_order_to_gateway`.

- Source: gateway `max_connections_per_user` config + playground
  `send_order_to_gateway`.
- Severity: **major** (blocks the canonical "hammer 100 orders"
  demo).

### F-N13 MAJOR — `/api/processes` reports `restarts=0` after pkill-and-supervisor-restart

`pkill -9 rsx-matching` → ME restarts → `/api/processes` shows
`restarts: 0`. The restart counter only increments for `mark`-style
supervisor-side restarts, not for "process died, supervisor noticed,
spawned new one" restarts. The /faults dashboard reports
`-60 (12 restarts)` in the health pill, so the count IS being
tracked elsewhere — the JSON field on `/api/processes` is just not
reading the right counter.

- Severity: **major**. Operators rely on this counter to know
  "did something just die in the last minute".

### F-N14 MODERATE — walkthrough still claims "11 crates · 2,550 tests"

Round-2 CEO flagged this. Today: `ls -d rsx-*/ | wc -l` = 15
(twelve cargo crates + three non-cargo subprojects); MEMORY says 887+
tests. Walkthrough hard-codes "11 crates · 2,550 tests · 54ns match
· 31ns WAL append". An investor with `grep` will catch it.

- Source: `rsx-playground/pages.py` walkthrough template.
- Severity: **moderate** (credibility hit, not functional).

### F-N15 MODERATE — `/docs` page loads Tailwind from CDN

`<script src="https://cdn.tailwindcss.com">` on the docs page (same
issue round-2 flagged for dashboard pages, not yet fixed for /docs).
Demo on an air-gapped laptop = unstyled grey wall of text.

- Source: `rsx-playground/server.py` docs response template.
- Severity: **moderate**.

### F-N16 MODERATE — `/x/pulse` says `proc 5/10` while `/api/processes` returns 7 entries

Pulse pill on dashboard: "proc 5/10". `scan_processes` returns 7
entries (gw-0, maker, mark, marketdata, me-pengu, recorder, risk-0).
The "10" is hard-coded somewhere (possibly counting BTC/ETH/SOL MEs
that don't exist in the `minimal` scenario). Two side-by-side
sources of truth disagree on the basic question "how many processes
do we have?"

- Severity: **moderate**.

## 5. Frustration-seconds for the canonical demo flow

| Step                                       | Time   | Outcome                                  |
|--------------------------------------------|--------|------------------------------------------|
| `./playground start`                       | < 1 s  | Server up.                               |
| `./playground start-all minimal` noconfirm | 0 s    | HTTP 400 — confirm header missing.       |
| `start-all` with `?confirm=yes`            | 22 s   | 6/7 running; mark blocked.               |
| Open `/walkthrough`                        | 3 ms   | Renders, "11 crates · 2 550 tests" wrong.|
| Open `/trade/`, switch to PENGU            | < 1 s  | Loads. Bid/Ask/Mark/Index all `--`.      |
| Open `/book` for PENGU                     | < 1 s  | 5-level synthetic ladder (fake).         |
| Submit one IOC buy                         | 2.0 s  | Green "accepted" (lying).                |
| `/x/live-fills?sym=PENGU`                  | < 1 s  | "no fills yet".                          |
| Click `Run one probe` on `/latency`        | 2.0 s  | `{"ok":false,"error":"timeout..."}`      |
| `/x/order-trace?trace-oid=<cid>`           | < 1 s  | "resting in book" (wrong for IOC).       |
| `rsx-cli dump .../10_active.wal`           | ~1 s   | `total: 0 records`.                      |
| Kill `marketdata` via /faults              | < 1 s  | Stopped. Stays stopped (no auto-restart).|
| Run `stress-low` 60s                       | 25 s   | submitted=0 → "PASS" in report file.     |

Aggregate: **the system is "responsive" in the sense that every
endpoint returns in <100 ms or 2 s sharp**, but the canonical
demo (submit → fill → verify in WAL) **cannot complete because the
matching engine processes zero orders**.

## 6. Verdict

Round-2 verdict was 22/100 ("the dashboard isn't on fire but the
first thing a counterparty would touch doesn't function"). Round-3
is **worse**: in round-2 the matching engine actually matched, the
WAL grew, fills appeared on `/v1/fills`, the latency probe returned
15 965 µs. In round-3 the chain Gateway → Risk → ME is silent — the
matching engine accepts zero orders in 16 minutes, the WAL stays
zero bytes, the marketdata WS emits zero frames, the latency probe
times out 100% of the time, and the playground papers over the
breakage by **stamping every "accepted" response on a 2-second
timeout-as-success path**.

Three of round-2's five criticals regressed:

- **Trade UI**: round-2 broken (bundle/server schema mismatch);
  round-3 the schema mismatch is fixed but the page still shows
  `-- -- --` everywhere because no marketdata flows.
- **Ghost submissions**: round-2 critical; round-3 still critical,
  same code path (`server.py:4424` timeout-as-accept).
- **`/verify` lies green**: round-2 reported "4/7 processes" as
  PASS (fixed in round-3, "5/8 → FAIL"); but the page now PASSes
  "34 orders no duplicates" while the live WAL is 0 bytes, and
  PASSes "WAL stream btc has files: 1 files, 0.0B" for symbols
  with no ME.

The walkthrough page still over-claims (`11 crates · 2 550 tests`
vs reality `12 crates · 887 tests`). The mark daemon panics out of
the box because nobody calls `CryptoProvider::install_default()`.
The book ladder on `/book` is synthesised from the maker's *desired*
mid-price, not exchange state, with no badge to disclose this.

**Score: 14 / 100.**

Below round-2's 22 because round-2 had a working matching path (you
could *show* fills happening); round-3 has nothing matching at all,
and the dashboard's response is to lie green about it. A friendly
first counterparty given 30 minutes with this build would conclude
the team is shipping demoware on top of a broken exchange and gilding
the failures with green checkmarks.

**Call: not yet.**
