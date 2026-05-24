# CEO Review Round 2 — RSX (2026-05-24)

Reviewer persona: founder / growth-stage CEO. Tools: live
browser + curl against `http://localhost:49171/`. No source,
no specs, no architecture docs. Reports what a first-time
investor / customer / partner would see in 5-15 minutes of
clicking. Adversarial by design; the strengths list exists
only to flag what *not* to break, not to soften the
criticism.

Round 1 verdict (carried from memory): **NO greenlight** —
five critical findings (raw i64 on /risk, dashboard
self-thrash, Trade UI loading forever, /verify "no trades
yet" while fills happened, CMP 1117/1117/1117 ghost). 28
fixes (F1–F28) were shipped between rounds.

## 0. Round 1 → Round 2 diff (like-for-like)

| Round-1 critical                                                | Round-2 verdict        | Evidence (UI)                                                                                                                                            |
|-----------------------------------------------------------------|------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------|
| Raw i64 rendered as USD on /risk                                | **CONFIRMED-RESOLVED** | /risk shows uPnL `-$19.60`, notional `$9,994.80`, IM `$999.48`, MM `$499.74`. (screenshot 01)                                                            |
| Dashboard self-thrashes (75s `/x/health`)                       | **CONFIRMED-RESOLVED** | `/x/health` is ~1 ms warm, ~3 ms cold across 5 trials. Health pill updates within 1 s of a kill. (screenshots 05/06/07)                                  |
| Trade UI loading forever                                        | **REGRESSED / STILL BROKEN** | `/trade/` shows `Loading... ▾`, `--Bid --Ask --Mark --Index` after 5 s. Pair-selector listbox is empty. JS bundle expects `{M:[[id,tick,lot,name]]}`, server `/v1/symbols` returns `{symbols:[{…}]}`. Schema mismatch — bundle is the wrong build for the API. (screenshot 04, 08) |
| `/verify` says "no trades yet" while fills happened             | **PARTIALLY-RESOLVED** | "Fills precede ORDER_DONE" check now PASSes with `2431 fills, 0 seq inversions`. But "Exactly-one completion per order" still SKIPS with `no completed orders observed`, and the WAL timeline visibly contains ORDER_DONE rows. Two subsystems disagree. (screenshot 03) |
| CMP `1117/1117/1117` ghost on /topology                         | **PARTIALLY-RESOLVED** | Numbers no longer lie about traffic — they now read `0/0/0/0` on every link (Gateway→Risk, Risk→ME, ME→Mktdata) despite 170 000 WAL events, 3 890 maker orders, an active /stress run. The dashboard's truth function is "show zero" rather than "lie loudly". Different bug, same broken metric. (screenshot 13/14) |

Net: 2 of 5 round-1 criticals confirmed-resolved, 1 still
broken, 2 partially-resolved. Round 1's score was 0/100
("would not greenlight"). Round 2's score: see Verdict.

## 1. Verdict

**Would I greenlight RSX to an outside investor or first
paying customer next week? No. Confidence: 22/100.** Up from
0/100 in round 1, but still firmly NO.

The single biggest reason: **the demo flow that a CEO would
actually drive on a sales call does not work end-to-end.**
The /trade UI — the page literally labelled "Trade" in the
top nav, and the first thing any non-engineer visitor would
click — shows "Loading… ▾" with empty price fields after 5+
seconds. The "Sign in with GitHub" button returns 502 from
`/auth/github` because the auth service is dead. The
playground's own `/orders` page accepts a click and stamps
"sent", but never transitions to "filled", never shows the
order's user position, never displays a latency number, and
its `Order Lifecycle Trace` says "pending — awaiting
gateway" 30 seconds after the order was already settled at
the matching engine.

You can recover from that on a technical-DD pitch ("the
engine works, look — WAL has 170 000 records, /risk has
positions in dollars, latency probe is 16 ms") — but you
cannot recover on a customer pitch, because the cleanest
five-click demo (open the playground → click Trade → see
prices → place an order → watch it fill) fails on click 3.

Round-1's "no" was triage-style ("the dashboard is on
fire"). Round-2's "no" is product-style ("the dashboard
isn't on fire, but the first thing my counterparty would
touch doesn't function as advertised"). That's an upgrade,
but not a greenlight.

## 2. Top 5 strengths (don't break these)

1. **Walkthrough page narrative is well-written and lands
   the wedge in <60 seconds.** Headline reads "Spec-first
   perpetuals exchange. Fixed-point. Single-threaded
   matching. 54 ns match (measured); <50us round-trip
   budget." The section structure (Big Picture → Order
   Lifecycle → Matching → Risk → WAL → Market Data → Mark
   Price → Numbers → Try It) is a complete pitch deck in 9
   headings. (UI: `/walkthrough`, screenshot 00.) Do not
   collapse all sub-sections behind `Expand details` —
   investors will skip them; un-collapse the first one.
2. **`/risk` dashboard is now a real risk page.** Two
   PENGU positions (long+short, each 20 000 qty),
   per-position uPnL/notional/IM/MM in USD with the correct
   sign, funding rate row, "5h45m" countdown to next
   funding. This is the page that single-handedly fixed the
   round-1 verdict — it now looks like Hyperliquid or dYdX
   on a quiet day. (UI: `/risk`, screenshot 01.)
3. **Fault injection page is investor-grade.** `/faults`
   lists 7 processes with `Stop` / `Kill` / `Restart`
   buttons, kills register inside 300 ms on the health
   pill, the score moves 70 → 55 → 40 → 25 as you take
   down ME, GW, and Mark sequentially, the colour rolls
   `yellow → red` honestly, and the reason text
   (`-15 (1 stopped) · -30 (186 error lines)`) is good
   enough to demo to a CTO. (UI: `/faults`, screenshots
   05/06/07.)
4. **Page load times are uniformly excellent.** Sixteen
   top-level navigation targets all serve under 4 ms warm,
   max 173 ms cold (`/orders` first hit). Static assets
   (the 433 KB `/trade` JS bundle) serve in 4 ms. There is
   no perceived "loading" cost for the dashboard frame
   itself. (Timing log in §7.)
5. **`/verify` is a real correctness page, not theatre.**
   16 invariants listed by name, including "Fills precede
   ORDER_DONE", "Position = sum of fills", "Funding
   zero-sum", "Advisory lock exclusive". Each row shows
   `PASS/SKIP/FAIL` with a one-line reason ("`2431 fills,
   0 seq inversions`", "`positions match fill sums`",
   "`7 services, no duplicates`"). The page even surfaces
   one FAIL — "WAL self-consistency (shadow vs WAL BBO)
   1/1 mismatch" — which is a real CTO finding that
   another exchange demo would have hidden. (UI: `/verify`,
   screenshot 03.)

## 3. Top 5 NEW risks (forced rank, NOT in F1-F28)

1. **CRITICAL — `/trade/` is fundamentally broken: schema
   mismatch between the React bundle and the server's
   `/v1/symbols` response.** The bundled JS expects
   `(await Tn("/v1/symbols")).M.map(([e,n,s,r])=>({…}))`
   — i.e. an `{M:[[id,tickSize,lotSize,name]]}` tuple
   format. The server returns
   `{"symbols":[{"id":1,"symbol":"BTC","tick_size":50,…}]}`
   — keyed objects. The pair-selector listbox is therefore
   empty, the header BBO/Mark/Index/Funding fields all show
   `--`, and the page sits at `Loading... ▾` forever. This
   is not a transient — it is reproducible 100% across
   page reloads. The Trade UI is the only page in the nav
   that an outside customer would naturally land on first;
   it is the only page that does not function at all.
   (UI: `/trade/`, screenshots 04/08.) **Severity: critical.**
2. **CRITICAL — order POST returns 200 OK even when the
   matching engine and gateway are both killed.** Killing
   `me-pengu` and `gw-0` via `/faults`, then submitting a
   buy order via `curl -X POST .../x/order` returns
   `HTTP/1.1 200 OK` with body `no data`. The user has no
   way to know the order vanished into a void. The
   `/orders` UI shows the row as `sent`, never advances.
   Round 1 complained about ghost-fills; round 2 has
   ghost-submissions. (UI: `/orders`, `/faults`,
   manual curl.) **Severity: critical.**
3. **CRITICAL — `/x/*` namespace is a wildcard sinkhole
   that returns `HTTP 200 + "no data"` for every unknown
   path.** `POST /x/literally_random_text` → 200 OK.
   `POST /x/order` (the legit one) → also 200 OK + "no
   data". This means the playground cannot distinguish
   "endpoint missing" from "endpoint returned empty" — and
   neither can the human reading the screen. Compare with
   `/api/foo` which correctly 404s. Two API namespaces
   (`/x/*` and `/api/*`) with opposite error semantics is
   itself a smell, but the wildcard 200 is a UX bomb.
   **Severity: critical.**
4. **IMPORTANT — `/verify` reports "RSX processes running
   4/7" as PASS when 3 of 7 processes are dead.** During
   the fault-injection run I killed me-pengu, gw-0, and
   mark. `/verify` Run All Checks then ran and stamped
   that very row as PASS at `10:19:20`. A correctness page
   that calls a 43%-down system "PASS" is worse than no
   page — it adds a confidently wrong signal. The same
   page's "Exactly-one completion per order" SKIPs with
   "no completed orders observed" while the WAL timeline
   visibly contains ORDER_DONE rows on the same screen.
   (UI: `/verify`, screenshot 03.) **Severity: important.**
5. **IMPORTANT — fault-injection `Restart` on /faults
   silently no-ops, but `Start` on /control works.** I
   killed `mark` from `/faults`, then clicked the
   `Restart` button next to mark three separate times.
   Each click returned "✓ Done" in the browser, the
   button never registered an error, but `mark` stayed
   `stopped` and the health pill stayed at 40/red. The
   same operation via `/control` → `Start` succeeded
   first try and the process came back in ~3 s. Two
   functionally different "restart" verbs on two pages of
   the same dashboard, where one of them lies about
   success. **Severity: important.**

## 4. Forced rank: 3 things to fix this week (with
acceptance test)

1. **Fix the `/trade/` schema mismatch and re-build the
   bundle.** Either change `/v1/symbols` to return the
   `M:[[id,tickSize,lotSize,name]]` tuple format the
   current bundle expects, or rebuild the bundle against
   the keyed-object shape. **Acceptance test:** open
   `/trade/` in an incognito window, the pair-selector
   button shows `PENGU` (not `Loading... ▾`), the bid/ask
   header shows numbers (not `--`), within 3 seconds of
   page-load and without clicking anything. While you're
   there, decide whether the `Sign in with GitHub` link
   should be visible when `rsx-auth` is down (right now
   it's a 502 hyperlink trap).
2. **Make `/x/order` fail loudly when matching/gateway is
   not reachable, and remove the `/x/*` wildcard 200
   sinkhole.** `/x/*` should 404 unknown paths exactly
   like `/api/*` does. `/x/order` should return either an
   accepted `oid` or a non-200 with a reason. **Acceptance
   test:** with `me-pengu` and `gw-0` killed via
   `/faults`, `curl -X POST .../x/order -d 'symbol=PENGU&
   side=buy&qty=10&tif=IOC'` returns HTTP 503 (or 502)
   with a body like `{"error":"no matching engine for
   PENGU"}`. Also `curl /x/literally_random_text` returns
   HTTP 404.
3. **Reconcile `/verify`'s subsystem disagreement and
   demote misleading PASSes.** "RSX processes running
   4/7" must be FAIL when 4 < 7 — count and threshold
   should live in the same check. "Exactly-one completion
   per order" must consume the same WAL timeline that the
   `/wal` page consumes — they cannot disagree on whether
   ORDER_DONE records exist. **Acceptance test:** kill
   any process; click Run All Checks; the row
   "RSX processes running" goes red FAIL. With the system
   healthy and at least 10 ORDER_DONE rows in the WAL
   timeline, the "Exactly-one completion" row goes PASS
   with a count (not SKIP with "no completed orders
   observed").

## 5. Surprises

Positive:
- The latency probe button on `/latency` actually runs an
  end-to-end probe and returns a real number (15 965 µs
  for me, with the raw JSON shown:
  `{"ok":true,"elapsed_us":15965,"cid":"probe-9387943",
  "oid":"019e598109ae7001ade2da93eee87ccb",…}`). For a
  CEO demo this is the strongest "show, don't tell"
  moment in the whole product. It's hidden behind a
  manual "Run one probe" button rather than running
  continuously, which is the right default for a dev tool
  but the wrong default for a demo.
- The walkthrough copy on the home page leads with
  numbers (`54 ns match`, `<50 us round-trip budget`)
  instead of marketing adjectives. Rare and good.
- `/verify` voluntarily surfaces a FAIL ("WAL
  self-consistency 1/1 mismatch"). Most exchange demos
  hide their FAILs. This is on-brand for a spec-first
  project and it should be the headline feature.

Negative:
- "11 crates · ~2,550 tests · 54ns match · 31ns WAL
  append" — the walkthrough literally says 2,550 tests
  and 11 crates. The MEMORY snapshot we carry between
  rounds says 12 crates and ~887 tests. One of those
  numbers is fiction; either way the marketing math
  doesn't reconcile with the project state, and an
  investor with a sharp pencil will catch it in 30
  seconds.
- `/topology` shows `Gateway 0 fills (session)` and "CMP
  CONNECTIONS Gateway→Risk SENT 0 RECV 0 NAK 0 DROP 0"
  while a stress test is mid-flight and the WAL is
  growing by 30+ records/second on the same screen. The
  numbers aren't lying loudly any more (round 1's
  `1117/1117/1117`) but they're lying quietly (`0/0/0/0`).
  Quiet lying is worse for credibility than loud lying.
- Uptime renders as `2m60s` (should be `3m0s`) in the
  process table on `/walkthrough`. Tiny, visible.
- "GW: checking..." sits on `/overview` for ~3 s after
  page load before becoming "GW: live". On every reload.
  Investor sees "checking..." every time they bounce.
- The home page redirects `/` → `/walkthrough`, then
  `/walkthrough` is the same page that shows the
  process-control PANEL with `Start All` / `Stop All`
  buttons. Mixing the marketing narrative ("Spec-first
  perpetuals exchange…") with `Stop All` admin buttons
  on the same screen makes it visually unclear who the
  page is for.
- `/orders` "Order Lifecycle Trace" says "pending —
  awaiting gateway" for an order that completed long
  ago. Same page also calls the textbox `oid…` but
  accepts a `cid` (the table column is labeled `CID`).
  Field-name confusion compounds: trace lookup never
  resolves because cid≠oid and there's no error.
- The "STALE ORDERS (>1 HOUR UNFILLED)" widget shows
  "0 stale orders" — that's because the playground's
  notion of "my orders" is *just the orders I clicked
  this session*. The maker's 3 890 orders aren't yours,
  the historical WAL's tens of thousands of orders
  aren't yours, "stale orders" is structurally always 0.
  The widget is doing nothing.
- The "MARGIN LADDER" widget on `/risk` shows 20+
  identical rows: `PENGU buy 0.050049 qty=10 notional=
  5004.9`, repeated. There is no aggregation, no
  per-price grouping, no per-user grouping — it's a raw
  dump of the maker's level quotes.
- `/wal` timeline renders prices as raw fixed-point:
  `sell px=50147 qty=1400000`. The column is honestly
  labelled `DETAIL (PX/QTY RAW)`, so this is a
  documented choice — but every other page (Book, Risk,
  Trade) formats fixed-point as the human price. Round
  1 said the i64 leak on `/risk` was a critical; the
  same leak exists on `/wal` and is just labelled
  `RAW` to make it OK. CEO finding: an investor scrolling
  /wal will not read the column header.
- `/v1/fills?sym=PENGU` returns
  `{"detail":"1 validation error: … Input should be a
  valid integer, unable to parse string as an integer …
   File \"/home/onvos/sandbox/rsx/rsx-playground/
   server.py\", line 7675, in v1_fills"}`. Two
  problems: (a) the public API rejects symbol names and
  wants integer sym_id, contradicting the symbol
  selector on `/book` that lists "PENGU SOL BTC ETH";
  (b) the 422 leaks the absolute source path of the
  developer's server file. Source path leak is the kind
  of thing a sec-review CSO would not let you ship.
- The `/stress` page reports "No stress tests run yet"
  during and after I ran a `stress-low` scenario for
  ~60 s. The scenario indicator went `● running` then
  back to `○ idle`, but the historical-reports table
  never gained a row. Either the report writer is
  broken or the page never reads from it.
- The maker page shows `Last error: gw: Cannot connect
  to host localhost:8080 ssl:default [None]`. Two
  problems baked into one error string: (a) port 8080
  isn't the gateway's port in this scenario; (b) `ssl:
  default` on localhost is a misconfiguration. Maker
  still reports `Status: running, Orders placed: 3890`
  on the same screen — so it's quoting OK but its
  internal config is wrong. Investor with engineering
  background will pull on this thread.
- "Restarts 0" on the maker page despite logs showing
  "[maker] quote circuit breaker: 10 consecutive
  errors; aborting maker" *twice*. The maker page is
  counting hard-PID restarts but not its own internal
  circuit-breaker aborts.
- `/overview` shows `proc 10/10` and `/x/pulse` shows
  `proc 9/9` on the same machine at the same time. The
  inflation seems to count the maker thrice (or the
  recorder is somehow split). Two side-by-side widgets
  on the same dashboard cannot agree on a basic count.
- The latency page references `.ship/12-SHOWCASE-HONEST`
  in its prose ("queued as task F1 in
  `.ship/12-SHOWCASE-HONEST`"). That directory was
  pruned in commit 9728dcf per the diary. Stale doc
  reference visible to anyone reading the page.

## 6. Out-of-scope notes (CTO domain)

Code smells visible from the UI alone:
- The reviewer-CEO can see two HTTP namespaces with
  opposite error semantics (`/x/*` swallows unknowns
  to 200, `/api/*` 404s them). This is an architecture
  call that leaked into UX. Pick one.
- The Trade UI bundle is built against an API shape
  the server doesn't serve. Either the FE and BE are
  on different commits, or the FE was vibe-coded
  against a spec that was later changed in BE. Either
  way, build artifacts are out of step.
- `/verify` exposes a real FAIL ("WAL self-consistency
  shadow vs WAL BBO 1/1 mismatch"). That is the
  *kind* of test we should be running, and it caught
  a real bug. The fact that the same page reports
  "RSX processes running 4/7" as PASS when it shouldn't
  suggests the test framework has a per-row severity
  bug, not just one bad assertion.
- `me-btc` shows 235.9% CPU on `/topology`'s process
  list. The spec ("single-threaded matching engine")
  vs the observation ("multi-core busy loop") is
  either a tile-thread artifact (fine) or a busy-spin
  leak (not fine). Worth one engineer-hour to confirm.
- `/auth/github` returns 502 with body `{"error":
  "rsx-auth not running"}`. The auth service should
  either be in the default start-set, or the "Sign in
  with GitHub" link should not be present until the
  service responds. Currently it's a confidently
  broken hyperlink.
- `recorder` logs in the unified error stream:
  `BLOCKED: 21 consecutive stream errors exhausted
  retry budget (20)`. The fact that this stays in
  the error scroll for hours (timestamp 2026-05-23T14)
  suggests retention is correctly long, but the
  recorder is sitting in a permanently-failed state
  with no recovery — and there's no UI affordance to
  "clear" it or "re-arm" it.
- Tailwind is loaded from `https://cdn.tailwindcss.com`
  on every dashboard page. Offline demo (e.g.,
  customer's locked-down laptop) = unstyled page. For
  a "spec-first" project this is the cheapest
  vendoring fix in the world.

## 7. Frustration-seconds aggregate

Page-load time across the 16 top-level nav entries (warm,
3rd attempt):

```
/walkthrough     0.001 s    /control     0.001 s
/overview        0.001 s    /maker       0.001 s
/topology        0.001 s    /faults      0.001 s
/latency         0.001 s    /verify      0.001 s
/book            0.002 s    /orders      0.001 s
/risk            0.002 s    /stress      0.001 s
/wal             0.002 s    /docs        0.003 s
/logs            0.001 s    /trade       0.002 s
                                         ──────
                            Σ            0.025 s
```

Page-frame budget for a complete dashboard tour: **25 ms**.
That is best-in-class. If the only metric mattered were "is
the dashboard responsive" the score would be 90/100.

The frustration is not in page-frames. It is in *what
happens after you click something*:

| Action                                        | Wait          | Outcome                                                       |
|-----------------------------------------------|---------------|---------------------------------------------------------------|
| Open `/trade/` and wait for prices            | 5+ s          | Still shows `Loading… ▾`, `--Bid --Ask`. Bundle/server mismatch. |
| Click `Sign in with GitHub`                   | ~1 s          | 502 from `/auth/github`.                                       |
| Click `5x BUY` on `/orders`                   | ~0.5 s        | Row appears as `sent`, never advances.                         |
| Lookup the trace for that order               | ~1 s          | "pending — awaiting gateway", forever.                         |
| Wait for `/orders` row to show fill latency   | ∞             | Latency column stays at `-` until reload.                      |
| First `/orders` cold load (one-off)           | 173 ms        | Tolerable.                                                     |
| First `/x/order` POST cold (one-off)          | 455 ms        | Tolerable but visible.                                         |
| Click `Restart` for `mark` on `/faults` x3    | ~1 s × 3      | All "✓ Done", mark stays dead. No error surfaced.              |
| Click `Start` for `mark` on `/control`        | ~3 s          | Works.                                                         |
| Click `Run All Checks` on `/verify`           | ~1.5 s        | Works; 6 SKIP, 1 FAIL surfaced.                                |
| Click `Run one probe` on `/latency`           | ~3 s          | Works; returns 15 965 µs E2E.                                  |
| Click `▶ start` on `/stress` low scenario     | ~10 s before any visible feedback on the stress page itself | Did start, but the stress page never shows in-progress stats; only `/overview` log stream proves it. |

**Aggregate frustration-seconds for the canonical demo flow**
(open Trade → see prices → place an order → see fill →
verify position on /risk → drill to WAL):

- 0 s opening pages (25 ms total dashboard)
- ∞ s on the "see prices" step (Trade UI never loads)
- Demo cannot proceed past step 2 without falling back to
  the playground's `/orders` page.

Fallback demo on `/orders`:
- 0 s opening
- 0.5 s clicking 5x BUY
- ∞ s waiting for the row to show fill state
- 0.5 s pasting the cid into trace
- ∞ s waiting for trace to say anything other than
  "pending — awaiting gateway"
- ~3 s to visit /risk and see *the maker's* positions
  (not yours)
- ~1 s to visit /wal and find raw `px=50147 qty=1400000`

**Total perceived wait on the canonical demo: unbounded.**
The dashboard is fast; the *system the dashboard
represents* leaves the user staring at "sent" / "pending"
/ "Loading…" forever, on the three most-clicked pages.

---

## Executive summary (≤250 words)

**Verdict: 22/100, NO greenlight, up from 0/100 in round 1.**

What round 1 fixed: the dashboard no longer self-thrashes
on `/x/health` (was 75 s, now 1 ms). The raw i64 bug on
`/risk` is gone — positions read in proper USD with sign,
notional, IM, MM. The `/verify` page exists and runs
real invariants, including one PASS row with `2431 fills,
0 seq inversions` and one voluntarily-surfaced FAIL on
WAL self-consistency. The walkthrough narrative leads
with measured numbers (54 ns match, <50 µs round-trip
budget) rather than marketing.

What round 1 did not fix, or made worse: `/trade/` — the
nav entry literally named "Trade" — still shows
`Loading… ▾` with empty bid/ask/mark/index, because the
React bundle expects `{M:[[…]]}` tuples and the server
serves `{symbols:[{…}]}` objects. The `/orders` form
accepts clicks but never shows a row's lifecycle past
`sent`. Order POST returns 200 OK even with the matching
engine *and* gateway both dead — ghost-submissions
replace round-1's ghost-fills. `/x/*` is a wildcard
sinkhole that 200s every unknown path. `/verify` calls
"4/7 processes running" a PASS. `/faults` `Restart` lies
about success while `/control` `Start` works. Maker page
shows config-broken error string (`localhost:8080
ssl:default`) yet "running". CMP traffic counters read
`0/0/0/0` while WAL grows by 30+ events/sec.

Five round-2 critical findings, zero of which were in
the F1–F28 set. Three are blocker-grade. The dashboard
shell is best-in-class; the system underneath does not
support the demo flow advertised on the front page.
