# CEO Review — RSX (2026-05-22)

Live operator dashboard at http://localhost:49171/, ~5 hours of cluster uptime,
maker bot grinding, no human traffic. Walked every tab, drove the cluster
(killed recorder, ran a stress, submitted orders, switched scenarios). What
follows is the unvarnished view from the chair that signs term sheets.

## 1. Verdict

**No, I would not greenlight this for fundraising or a customer demo today.**

Single biggest reason: the dashboard's first paint shows a wall of
"loading..." panels with "GW: offline" in the corner while the system
claims health 70/YELLOW. The "no longer lies" claim does not survive
sixty seconds of live use. /x/health responds in **75 seconds** under
self-poll load; /x/key-metrics in **15s**; meanwhile the maker has
spammed 77,000 orders without producing a single user-facing demo
moment. A Series-A partner looking at this for five minutes will not
see "sub-microsecond match" — they will see a dashboard that is itself
slow, raw fixed-point integers rendered as USD margin numbers, an
orphaned "FAIL" reconciliation row with no drill-down, and a Trade UI
that is permanently "Loading...". The engineering is plausibly real;
the *product* is not assembled.

I would book one more 4-week refine pass before showing this to anyone
who isn't already aligned.

## 2. Top 5 strengths (don't break these)

1. **/walkthrough has an actual story.** The hero shows the architecture
   ASCII diagram, the orderbook ladder reads cleanly with human prices
   ("0.0501 / 0.0499 / spread 0.0002"), and the eight section headings
   (Big Picture → Order Flow → Matching → Risk → WAL → Market Data →
   Mark Price → Numbers → Try It) are a defensible narrative. **This
   is the page that could sell.** Protect it.
2. **/latency is unusually honest for an HFT-adjacent project.** It
   explicitly says "The gateway round-trip card above measures the
   playground ↔ gateway path... It overstates the matching latency —
   it's useful as a liveness signal, not as the <50 µs claim." That
   single paragraph buys credibility with technical investors who are
   sick of latency theater. Keep it verbatim.
3. **/book is the most polished read-only surface.** Orderbook ladder
   for PENGU is correct, BID/ASK colored, spread shown, "BOOK STATS"
   per symbol, "LIVE FILLS" tail with seq numbers. This is what every
   other tab should aspire to.
4. **/faults works and health actually responds.** Killed `recorder`
   via the UI → health dropped from 70 to 55 within one poll, the
   process row went red with PID "-" and the Stop button flipped to
   Start. Restart restored it. The reactivity is real (against the
   feared "dashboard prints green while ME is restarting" failure
   mode).
5. **/verify lists ten named invariants with PASS / SKIP / FAIL and
   substring rationale.** "Funding zero-sum across users per symbol —
   all symbols net funding = 0" is exactly the kind of line a serious
   exchange ops team wants to see. The fact that some SKIP with
   honest reasons ("requires slab metrics export (not yet wired)") is
   a trust signal, not a weakness.

## 3. Top 5 risks (forced rank)

### R1 (critical): Raw fixed-point integers are rendered as USD values
on /risk, /maker, /topology drill-downs, and /wal timeline.

`/risk` shows:
- "user 1 — COLLATERAL 999999972019150 — EQUITY 999999860319150 — UPNL
  -111700000 — IM REQUIRED 5585000000 — MM REQUIRED 2792500000".

There is no decimal point, no unit, no comma separator. This is the
output of an i64 fixed-point ledger leaked into the UI. The same
page shows `user 1` notional "5585000" in one column and "NOTIONAL
5,585,000" in another — different scales on the same row.

`/topology` "Matching" drilldown shows `sym10 bbo bid=49900 ask=50100
spd=200` while `/book` for the same symbol shows `0.0499 / 0.0501 /
spread 0.0002`. Two windows, two answers, one symbol.

`/maker` shows `mid prices: sym10=50000` (raw) while landing page
shows `mid ~0.05`.

**Why this kills funding**: every quant or exchange-ops investor will
see this and conclude the team has never sat with an actual trader.
This is the single most common "founders haven't shipped to users"
tell on the entire dashboard.

### R2 (critical): "No longer lies" claim is half-fixed.
- /x/health reacts to process state (good — R1 from prior audit landed)
  but does NOT degrade to RED when **/verify shows a FAIL row**
  ("WAL self-consistency (shadow vs WAL BBO) 1/1 mismatch"). System
  with a failing invariant is still "70 YELLOW".
- /x/health reports YELLOW=70 with header banner "GW: offline" visible.
  Gateway offline + all-green health = the audit's flagship lie pattern
  is not closed.
- /x/cmp-counters reports "Gateway → Risk 1117, Risk → ME 1117, ME →
  Mktdata 1117" — three different hops, identical numbers, after 4h
  of maker activity (77k orders placed). These are not real cluster
  counters. They are a single counter copied three times, or every
  hop happens to have processed exactly the same 1117 frames since
  some reset.
- /x/latency-regression has been "--" on every visit; the bottom card
  on /latency says `GW->ME->GW p99 --` despite n=619 e2e probes
  measured above showing p99=225ms.

### R3 (critical): Dashboard self-thrashes under its own polling load.
Endpoint timings I measured directly with curl:

```
health           75.0s
pulse            16.0s
key-metrics      15.5s
logs-tail        15.4s
processes         0.25s
wal-status        0.27s
invariant-status  0.53s
```

A user opening /overview sees every panel saying `loading...` for
8+ seconds, then partial loads, then more loading. The "Loading..."
overview screenshot is what an investor sees during the first 8
seconds of a demo. There is also "GW: checking..." for the entire
load cycle. (Screenshots: 02b-overview-zoom.png through
02f-overview-after.png — every one captured mid-load.)

### R4 (important): /trade is a dead UI under the same domain.
Public-facing React app reachable at `/trade/` shows "Loading... ▾"
for the symbol selector, "Bid -- Ask -- Mark -- Index --" across the
whole ticker, "connecting --" with a yellow dot in the header, "No
recent trades" in the trade list, no candles on the chart (only
volume bars on a broken -20 to 0.08 Y-axis), and "Sign in with
GitHub" CTA. This is the surface most prospective integrators will
actually visit. It looks like the API contract between Trade UI and
the live exchange is broken.

### R5 (important): The dashboard advertises a 4-symbol exchange
but only PENGU has data. PENGU/SOL/BTC/ETH selectors on /book all
work — selecting SOL/BTC/ETH yields `"BTC: no book data yet
(waiting for orders)"`. The maker only quotes PENGU. The topology
panel says "Marketdata: 4 sym (session)" — implying activity on
four — and Matching drilldown shows `sym1, sym2, sym3` all at "0b /
0a" depth, only sym10 (Pengu) live. The system markets multi-symbol
but ships single-symbol. Pick one.

## 4. Forced rank: if I could fix only 3 things this week

### #1 — Format every i64 in the UI through a tick_size-aware formatter.

**Why**: this is the cheapest, highest-leverage credibility move on
the dashboard. Today the team looks like it can build a matching
engine but cannot ship a balance display. Tomorrow it looks
user-facing-competent for the cost of two days.

**Acceptance test**: on `/risk` the collateral column reads
"$1,000,000.00" or "1,000,000 USDT" (whichever is real), not
"1000000000000000". On `/topology > Matching` the bbo string reads
"0.0499 / 0.0501 / 0.0002" not "49900 / 50100 / 200". On `/wal`
timeline the `px=50300 qty=1000000` entries are unchanged
(WAL is raw fixed-point — that's legitimate engineering surface, and
the column header on the row should say so: "px raw / qty raw").
The split is: operator-debug surfaces show raw; user-facing summary
cards show formatted with units.

### #2 — Fix the polling thundering herd so /overview paints in ≤500ms.

**Why**: every other CEO-level complaint flows from "the dashboard
feels broken on open". You can have the world's lowest-latency
matching engine — the front door says "loading...".

**Acceptance test**: from a cold browser open of /overview,
all six panels (System Health, Process Table, Key Metrics, Stats,
Ring Backpressure, WAL Status, Logs Tail, Invariants) show real
data within 1 second on a hot system. `curl -w '%{time_total}'
http://localhost:49171/x/health` returns in <200ms. Health endpoint
is not blocked on log-scanning a 200-line buffer.

### #3 — Wire one failure path end-to-end: failing invariant → red health.

**Why**: the "no longer lies" claim is the load-bearing trust signal
for this whole project. Right now /verify says FAIL on "WAL
self-consistency" and /x/health says YELLOW=70. Fix that one line of
arithmetic. While you're there, kill the "1117 / 1117 / 1117" CMP
counter ghost — that is the single most damning number on the page
because three obviously different pipes should not have identical
counters.

**Acceptance test**:
- /verify shows FAIL → /x/health goes RED (≤30 in score, "red"
  bucket), with the failing invariant name in the tooltip / hover.
- /topology CMP CONNECTIONS table shows three distinct values for
  three distinct hops, and the values move when I run a stress test.
- When mark process is genuinely down, the "Book mid vs mark-process
  index" check status agrees with the topology "Mark" node color
  (both say down, not one running 60% CPU while the other says "no
  index").

## 5. Surprises

### Positive
- **Topology component drilldown works for 7 of 8 nodes.** Clicking
  Risk, Matching, Mark, Recorder, Maker, Marketdata, Clients all
  produce a sensible mini-panel ("Risk: pid 516758, uptime 4h33m,
  funding next settlement 3h 30m, seed accounts 6"). That kind of
  drill-anywhere navigation is rare and good.
- **Verify reconciliation actually runs at click time** and reports
  a real FAIL ("WAL self-consistency: 1/1 mismatch"). Most "audit"
  pages on dashboards I've seen are static. This one isn't.
- **The Walkthrough page is built like a sales narrative,** with
  anchor links across the top ("Big Picture / Order Flow / Matching
  / Risk / WAL / Market Data / Mark Price / Numbers / Try It") and
  expandable `<details>` blocks under each heading containing the
  actual ASCII architecture diagram. Cleanly thought-through.
- **Maker is genuinely working.** 77k orders, 10 active, 0 restarts,
  spread 20 bps, mid 0.05. The exchange does match — invariants and
  the live fills stream confirm it. The product is not vapor.

### Negative
- **The "errors" pulse counter on /overview shows 208 (then 189, then
  204) but there is no link from that number to a filtered log.**
  When I clicked "errors only" on /logs I found the underlying noise
  is repeating since 11:21:09: "config poll failed", "snapshot save:
  No such file or directory", "stream error (14/20): Connection
  refused" all the way to (20/20). The recorder retried 20 times and
  died, was restarted, and is still spitting. 208 errors is a
  perpetually-degraded state that nobody addresses because there's no
  drill-down workflow.
- **"unknown component: summary" text is visible on every /topology
  page load** above SELECTED COMPONENT. Looks like a stray render of
  an unmatched route case. Visible bug.
- **/orders is broken end-to-end.** Submitted "SELL 1x" → row appears
  with Price "0", Qty "100000", Status "sent", Latency "-". Six
  seconds later, still "sent" and "-". GET `/api/orders/<cid>`
  returns 404. The maker has placed 77k orders but my orders just
  vanish.
- **The Stress tab "HISTORICAL REPORTS: No stress tests run yet"**
  after my form-submitted 5s/50ops stress test completed. The server
  returned `{"code":"OK","message":"stress started"}` as a raw JSON
  blob inserted into the page — no result panel ever populated. The
  prebuilt scenario buttons ("stress-low / stress-high / stress-ultra")
  also have no progress indicator; the radio circles next to each
  ("○") never change.
- **/control silently reboots the cluster when you switch scenarios.**
  Clicking "Switch Scenario" with stress-low selected restarted 7 of
  9 processes (uptimes went from 4h33m → 2m26s) and added two new
  ME processes (me-btc, me-sol) at ~50% CPU each, all on the same
  page load. No confirm dialog, no "are you sure". Live state change
  via single click. This is fine for a dev tool, but on a tab named
  "Control" with no separator from /overview, a customer demo with
  hands-on access ends in tears.
- **"11 crates · ~2,550 tests"** banner conflicts with project
  README's "12 crates" and the test count is the union of Rust +
  Python + Playwright with no breakdown.
- **Title bar inconsistency**: landing reads "RSX Exchange",
  overview reads "RSX Playground - Development Dashboard", docs read
  "RSX Playground", browser tabs read "RSX -- Walkthrough". Three
  names. Pick one.
- **/maker reports `Last error: order: [1005, 'order not found']`**
  as part of normal operation. That's an error in a "live stats"
  panel that should be empty when healthy.
- **/wal "archive" and "mark" streams both show 0.0B size for an
  entire 5-hour session.** Two of three WAL streams are zero. Either
  these processes aren't writing (which contradicts /verify PASSing
  "WAL stream mark has files") or they're churning empty files. No
  explanation in the UI.
- **/risk margin ladder shows 20 identical rows** ("PENGU buy 0.0501
  10 5010") — twenty rows that are byte-identical with no
  distinguishing field. Either the maker placed 20 of the same order
  (likely) or the table is replicating one row twenty times. Add a
  user_id or oid column.

## 6. Out-of-scope notes (cross-pollination to CTO)

Things I noticed while reading the UI that probably matter for the
engineering review:

1. **CMP frame counter is broken or fake.** Gateway→Risk = Risk→ME =
   ME→Mktdata = 1117 exactly. If this isn't a leaked single-process
   counter then the CMP probe is reading a stale snapshot. CTO should
   verify whether `/x/cmp-counters` is wired to real per-pipe metrics.
2. **Mark and ME processes burn 50-80% CPU at zero traffic.** Mark
   topped 116% during one observation. Three matching engines at
   ~60% each with zero orders flowing. Hot-spin loops? Polling
   without backoff?
3. **Recorder reliability**: in a single 5-hour observation the
   recorder went through 20 retries and died, restarted, and is now
   spitting WARNs on a schedule. /verify still says "PASS RSX
   processes running 7/7" (or 9/9 after switch) but the recorder is
   visibly the most fragile component.
4. **HTMX polling stampede**: every /overview panel poll triggers a
   per-process subprocess fork (ps, log scan, WAL stat). Latencies
   show 0.25s endpoints alongside 75s endpoints — disk-blocked log
   tails competing with cache-hot proc lookups. Endpoint catalog
   probably needs an in-process snapshot cache.
5. **Snapshot save failures every 3-4 minutes** ("snapshot save: No
   such file or directory (os error 2)") suggests a missing
   directory at startup. Easy fix, but it's been broken for the full
   audit window without anyone noticing because the noise is buried.
6. **Trade UI is on the same domain as the dashboard but cannot
   connect to the gateway.** "connecting --" never resolves to
   "connected". The handoff between WebUI and Gateway is the only
   path a customer will ever take. It does not work.
7. **The /verify "Tips monotonic, never decrease" SKIP says "no BBO
   records in WAL"** — but the /wal page lists 96k+ records of type
   ORDER_INSERTED / ORDER_ACCEPTED. If BBO genuinely isn't getting
   written then `make wal` correctness suite is incomplete. If it is
   getting written, the invariant check isn't finding it.
8. **The /risk "RISK CHECK LATENCY" panel shows P50/P95/P99/MAX all
   as "--".** Same shape as the broken regression panel. Likely no
   producer is writing to this metric.
9. **`/x/cmp-counters` returns the literal HTML string `<span
   class="text-slate-500 text-xs">no data</span>` from a different
   endpoint pathway than the topology view.** Two endpoints, two
   answers, same data.
10. **"recorder uptime 6m38s" when all others are 4h32m** — the
    recorder cohort died and restarted at 12:23 (matches "stream
    error 14/20...18/20" timeline in logs). This should have triggered
    a restart counter / alert. It silently kept the screen green.

---

## Executive summary (≤200 words)

RSX has the bones of a real exchange — 77k orders flowing through a
single-symbol matching engine, the orderbook ladder reads cleanly,
the Verify tab lists ten named invariants, and the Walkthrough page
has a defensible narrative. **But the dashboard is not assembled
into a product I would show to a customer in five minutes.** Raw
fixed-point i64 integers are rendered as USD on the Risk and
Topology pages ("COLLATERAL 999999972019150"). The Trade UI shows
"Loading..." and "connecting --" indefinitely. Half the /overview
endpoints take 15-75 seconds to respond, so the first paint is a
wall of "loading..." panels next to "GW: offline" in the header
while health claims YELLOW=70. The /verify FAIL row does not
degrade /x/health to RED. Three identical "1117" CMP counters across
three different pipes look like a leaked single-process metric. The
recently-closed "audit pass" left a working /faults page but did
not close the lying-by-omission failure mode. **One more 4-week
refine pass: format all numbers, fix the polling herd, wire one
failure path end-to-end. After that I would consider greenlight.**
