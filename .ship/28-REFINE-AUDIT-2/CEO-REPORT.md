# CEO Audit — RSX (.ship/28, 2026-05-28)

Reviewer persona: founder. Tools: live curl + log inspection against
`http://localhost:49171`. All 6 processes (gw-0, mark, marketdata,
me-pengu, recorder, risk-0) reported `state=running` at audit start.
Prior round score (.ship/27 CEO): **14/100**. Branch: HEAD at `e997598`.

---

## 0. Setup

- Server already running (started 10:52, ~30 min uptime at audit).
- `healthz` returns `{"status":"ok","processes_running":6,"processes_total":6,"postgres":true,"gateway":true,"marketdata":true}`.
- `mark` is running — no rustls panic this session. **F-N3 (critical) confirmed fixed.**
- `/x/pulse` shows `proc 6/6 ord/s 0 wal 5 errs 192`. The `192` errors are WARN lines counted from log files (see F-N29).

---

## 1. Did "Start All" start every process?

Yes. 6/6 processes running, health pill shows 6/6. Stark improvement
from round-3 where `mark` would panic and be abandoned.

---

## 2. Demo flow

### Submit order

Using correct lot/price alignment (PENGU: price_dec=6, qty_dec=4,
lot=100000 → `qty=10.0` = 100000 raw, `price=0.06` = 60000 raw):

```
POST /api/orders/test?confirm=yes  price=0.06 qty=10.0 side=buy tif=GTC user_id=1
→ order pg652dec3a54cca accepted (12015us)
```

Real end-to-end latency of 12ms. **F-N1 (critical, matching engine
silent) confirmed fixed.** The chain GW→Risk→ME now completes in
~12-15ms.

Note: the playground form in round-3 used `price=60000 qty=1000000`
(raw integers) directly, which triggered "price not tick aligned" and
"qty not aligned to lot" — the test endpoint now expects HUMAN-readable
values and handles the raw-to-int conversion internally. This is correct
but undocumented.

### Watch the fill

Submitted matching IOC sell:

```
POST /api/orders/test?confirm=yes  price=0.05 qty=10.0 side=sell tif=IOC user_id=2
→ order pg652dec4346646 accepted (14533us)
```

WAL grew from 224B → 672B, seq advanced to 6. `rsx-cli dump`:

```
{"type":"FILL","seq":4,"price":60000,"qty":100000,...}
{"type":"ORDER_DONE","seq":5,...}
{"type":"ORDER_DONE","seq":6,...}
total: 6 records
```

**Fill is real. WAL is written. End-to-end matching works.**

`/x/live-fills` confirms:
```
PENGU  sell  0.06  10  seq=2
```

### Verify WAL

`/api/verify/run-json` and `/api/verify/run` both **crash with
HTTP 500** (`KeyError: 'bid_px'`). See F-N28.

### Book after fill

```
GET /api/book/10  → {"bids":[{"px":60000,"qty":100000}],"asks":[],"source":"live"}
```

Live source, real data. But after filling, the book correctly empties on
sell side. **F-N4 (synthetic ladder) partially fixed** — when the
marketdata WS has data, `source=live` is served. The fallback to
`source=synthetic` still exists for cold-start.

```
GET /x/book?symbol_id=10  → crashes with KeyError: 'symbol_id'
```

See F-N27.

### CMP flow counters

```
GET /x/cmp-flows  → Gateway→Risk: 0/0/0/0, Risk→ME: 0/0/0/0, ME→Mktdata: 0/0/0/0
```

After 2 successful trades (WAL seq=10), flow counters are still
0/0/0/0. See F-N26.

---

## 3. Fault injection

### 3.1 Kill ME (pkill -9)

ME restarts in ~12s (supervisor pid 566792 ← was 215896). **Recovery
works mechanically.** Orders submitted during the gap return amber
`timeout: no matching-engine response in 2s` with HTTP 504 — this is
the **F-N2 fix** (no longer lies green). Correct.

After restart the book is restored from `snapshot.bin` + WAL replay:

```
book restored from snapshot: book.seq=1 wal_seq_sidecar=Some(6)
wal replay: accepted=0 cancelled=0 last_seq=6
```

Orders submitted post-restart flow through successfully (IOC fill
confirmed in WAL seq=8-10, total 16 records).

### 3.2 Restart counter still missing

```
GET /api/processes  → [...] NO "restarts" field in any entry
```

ME was killed by operator and restarted by supervisor — processes JSON
has no `restarts` key at all. **F-N13 (restarts=0 lie) not fixed;
worse: field removed entirely.** An operator cannot tell from the API
how many times a process has died.

### 3.3 GTC timeout is now honest (amber, not green)

When ME is restarting, GTC order returns:

```
order pg652dee08bfdd0 timeout: no matching-engine response in 2s
HTTP 504
```

Amber color, not green. **F-N2 fixed.** But GTC orders that successfully
rest in the book ALSO return the same amber timeout (because spec says
no ACK for resting GTC, per server.py comment at line 4448). A
visitor submitting a GTC order sees amber and can't distinguish
"resting" from "dead gateway." This is a known acceptable tradeoff
documented in the code comment, but it's confusing UX.

### 3.4 Malformed orders

| Input | Response |
|-------|----------|
| `price=60000&qty=1000000` (raw integers) | `price not aligned to tick` — NOT a bug, but silent to caller who read old docs |
| `price=0.05&qty=0.1` (wrong lot) | `qty not aligned to lot (100000)` ✓ honest |
| `side=sell&tif=IOC&price=0.04&qty=10.0` | `accepted (14ms)` — crosses against resting buy ✓ |

---

## 4. NEW findings (≥5 required; 6 below)

NEW = not in `.ship/27-REFINE-AUDIT/CEO-REPORT.md` findings F-N1 through F-N16.

---

### F-N26 MAJOR — `/x/cmp-flows` always shows 0/0/0/0 regardless of traffic

After 10 real WAL records (2 fills, 4 resting orders), every flow
counter is zero:

```
Gateway→Risk   Sent:0 Recv:0 NAK:0 Drop:0
Risk→ME        Sent:0 Recv:0 NAK:0 Drop:0
ME→Mktdata     Sent:0 Recv:0 NAK:0 Drop:0
```

Root cause: `parse_wal_records()` in `server.py:1631` uses
`WAL_HDR = struct.Struct('<HHI8s')` — the **V0 WAL header format**
(type:u16 at offset 0). Since commit `64dda88` the Rust code writes
**V1 headers** (version:u8 at offset 0, record_type:u16 at offset 2).

With V1 headers, the Python parser reads:
- `rtype = H[0:2]` = version(1) + pad(0) = **0x0001 = 1** (always, for every record)
- `rlen  = H[2:4]` = actual record_type value (0–13) used as payload length

So rtype=1 always, rlen=record_type_as_int. The `{RECORD_ORDER_ACCEPTED}` filter
(value 7) never matches because rtype is always 1, not 7. All flow
counters read zero permanently.

The same bug causes the `KeyError: 'symbol_id'` crashes in
`_latest_bbo_from_wal` (see F-N27) and `/verify/run` crashes
(see F-N28).

- Source: `rsx-playground/server.py:1578-1579` — comment says "type:u16, len:u16, crc32:u32, reserved:8" which is the stale V0 layout.
- Severity: **major**. Every WAL-based metric (flow counters, verify invariants, live fills from WAL) is silently broken.

---

### F-N27 MAJOR — `/x/book` crashes with `KeyError: 'symbol_id'`

`GET /x/book?symbol_id=10` → `<span class="text-red-400 text-xs">error: 'symbol_id'</span>`

Same WAL header bug (F-N26). When `parse_wal_records()` is called with
`{RECORD_BBO}` filter, rtype=1 matches (= RECORD_BBO), rlen=actual_type_value
(tiny), payload too small for BBO struct (72 bytes), so the `else:`
fallback at line 1754 appends:

```python
{"type": "bbo", "seq": 0, "ts_ns": 0}  # NO "symbol_id" key
```

Then `_latest_bbo_from_wal` does `sid = rec["symbol_id"]` → crash.

This crashes `/x/book`, `/api/book/{symbol_id}` (for WAL fallback
path), and the invariant-6 check in `/verify`.

- Source: `server.py:1754-1765` — else branch returns record without `symbol_id`.
- Severity: **major**. Book viewer and verify both broken.

---

### F-N28 MAJOR — `/api/verify/run` and `/api/verify/run-json` crash with HTTP 500

Both POST endpoints return `{"detail":"'bid_px'"}` (JSON routes) or
HTTP 500 (HTML route):

```
KeyError: 'bid_px'
File server.py, line 4791, in _run_invariant_checks
```

Same root cause chain as F-N27: the stale WAL parser returns a partial
record dict for invariant-5 (tips monotonic) that has `seq` but lacks
`bid_px`. Then invariant-6 (crossed-book) tries `r["bid_px"]` and
crashes.

The `/verify` page — the canonical proof-of-correctness showcase —
**cannot be rendered at all** whenever the WAL has been written to.
This is the single worst demo-blocker after the critical matching bug
was fixed.

- Source: `server.py:1578-1579` + `server.py:4820`.
- Severity: **major** (demo-blocker; the verify page is broken by design for any live system).

---

### F-N29 MODERATE — pulse shows `errs 192` from stale WARN lines in recorder.log

The pulse pill shows a persistent red `192` errors. These come from
`_count_recent_errors()` scanning the tail of log files for lines
containing ` warn ` or ` error `. The recorder log (`log/recorder.log`,
11k+ lines) accumulates WARN lines at ~20/minute:

```
WARN rsx_cast::replication_client: dxs: connect to 127.0.0.1:9110 failed: Connection refused
```

The recorder is mis-configured: `RSX_RECORDER_PRODUCER_ADDR=127.0.0.1:9110`
points to ME's **UDP casting port**, not a TCP replication port.
`RSX_ME_REPLICATION_BIND_ADDR` is not set for ME, so ME never starts
a replication TCP server. The recorder has been failing to connect
since May 25 (3+ days, 11 022 connection refused errors). It is being
restarted periodically by the supervisor (pid changed from 334761 to
557124 during this audit session).

Secondary issue: the `192` errors shown in the pulse is a
14-second-window tail, not "since boot." The badge stays red
continuously even when there are no real operational errors, which
trains observers to ignore it.

- Source: playground `start.py` (recorder env setup), `_count_recent_errors()`.
- Severity: **moderate** (alarming but non-blocking; desensitizes operators to error count).

---

### F-N30 MODERATE — stale pyc served: walkthrough shows "11 crates · ~2,550 tests" despite fix

The live walkthrough at `/walkthrough` renders:

```
11 crates · ~2,550 tests · 54ns match · 31ns WAL append
```

But `pages.py` line 650 (current on-disk version) says:

```python
12 Rust crates &middot; ~880 unit tests &middot;
```

The server loaded `pages.cpython-314.pyc` (mtime: May 24 23:01)
despite `pages.py` being newer (May 28 10:49). Python's timestamp
validation found a mismatch — pyc source mtime 1779647382 vs actual
1779965391 — and *should* have regenerated the bytecode, but did not.
The stale pyc was imported silently. The May 24 pyc still contains
the pre-.ship/28 strings "11 crates · ~2,550 tests".

The actual unit test count today: **374** (from `make test`), not 880
and certainly not 2,550.

Additionally: the stale `server.cpython-314.pyc` (May 24 23:49 vs
server.py May 28 10:51) explains the off-by-~30 line numbers in
server tracebacks — the server is running the May 24 version of
`server.py` logic.

- Source: `__pycache__/pages.cpython-314.pyc`, `__pycache__/server.cpython-314.pyc`.
- Severity: **moderate** (credibility hit + confusing tracebacks for debugging).

---

### F-N31 MODERATE — risk and mark at 93–99% CPU: sleep(100µs) busy-loop not fixed

At audit start, three processes pin CPU:

```
mark     running  92.5%
me-pengu running  92.5%
risk-0   running  93.4%
```

The sleep-audit (.ship/19) identified `monoio::time::sleep(Duration::from_micros(100))`
as a busy-poller in gateway (line 416 of `rsx-gateway/src/main.rs`)
and marketdata (line 405 of `rsx-marketdata/src/main.rs`). Neither fix
was applied. The CPU burn propagates: on a shared dev machine all
three cores hosting these loops saturate, adding latency jitter to
everything else.

- Source: `rsx-gateway/src/main.rs:416`, `rsx-marketdata/src/main.rs:405`.
- Severity: **moderate** (dev-box-only noise for now; would be blocking on bare-metal demo).

---

## 5. Regressions vs round-3 (.ship/27)

| Finding | Round 3 | Round 4 |
|---------|---------|---------|
| F-N1: matching engine silent | CRITICAL | **Fixed** |
| F-N2: timeout-as-accept lies green | CRITICAL | **Fixed** (now amber 504) |
| F-N3: mark rustls panic | CRITICAL | **Fixed** |
| F-N4: synthetic book ladder | MAJOR | Partially fixed (tagged `synthetic` when real data absent) |
| F-N5: latency-probe-gw ok=true + error_code=1007 | MAJOR | **Still present** (verified this session) |
| F-N13: restarts=0 lies | MAJOR | **Worsened** — field removed entirely |
| F-N14: walkthrough "11 crates · 2550 tests" | MODERATE | **Still present** (new root: stale pyc) |
| /verify crashes | N/A (worked in .ship/27) | **New regression** — crashes HTTP 500 (F-N28) |

---

## 6. Demo flow timing

| Step | Result |
|------|--------|
| `GET /healthz` | `{"status":"ok",...}` ✓ |
| Submit GTC buy (`price=0.06, qty=10.0`) | `accepted (12015us)` ✓ real |
| Submit IOC sell (`price=0.05, qty=10.0`) | `accepted (14533us)` ✓ fill |
| `GET /api/book/10` | `source:live, bids:[{px:60000}]` ✓ |
| `GET /x/live-fills?sym=PENGU` | Shows real fill at 0.06 ✓ |
| `GET /api/verify/run-json` | **HTTP 500 KeyError: 'bid_px'** ✗ |
| `GET /x/cmp-flows` | **0/0/0/0 (all zeros)** ✗ |
| Kill ME, wait, submit order | Amber timeout during restart ✓ honest |
| Post-restart order | `accepted (14873us)` ✓ |
| `GET /api/processes` | No `restarts` field ✗ |

The matching path works. The observability layer is broken.

---

## 7. Verdict

**Score: 51 / 100.**

Up from 14/100 in round-3. The three criticals from round-3 are all
fixed: matching engine processes orders (~12ms GW→ME→GW), timeout
responses are honest amber (not green), and mark no longer panics on
startup.

What is new and broken: the WAL Python parser (`WAL_HDR` struct) was
never updated for the V1 header format introduced in commit `64dda88`.
Every WAL-based metric is silently wrong. `/verify` crashes on any
live system. The flow-counter page shows 0/0/0/0 after confirmed
fills. A counterparty asking "show me the proof-of-correctness page"
gets an HTTP 500.

Three specific things to fix before the next CEO audit:

1. **Update `WAL_HDR` in `server.py`** to the V1 layout (`<BBHHHIBBBx4s` or similar) — fixes F-N26, F-N27, F-N28 in one change.
2. **Delete the stale `__pycache__/*.pyc` files** or ensure the playground start script clears them — fixes F-N30.
3. **Fix the recorder `RSX_RECORDER_PRODUCER_ADDR`** to point to a real TCP replication endpoint (or add `RSX_ME_REPLICATION_BIND_ADDR` to ME's env) — fixes F-N29.

**Call: not yet, but close.** The exchange itself matches correctly.
The dashboard layer has a structural defect (wrong WAL parser) that
makes every cross-check endpoint lie or crash.
