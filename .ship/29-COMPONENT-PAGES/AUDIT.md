# 29 — Playground UX Audit (13-yo lens, 4 browser agents)

System was live (6 procs) but **Postgres down** during audit, so
PG-backed numbers were degraded (noted where it mattered). All 15
parts rendered. Owner constraint: **no new walls of text — fix
with affordances** (tooltips, links, CTAs, cross-links).

## Real bugs (status after bugfix pass 2026-05-31)
1. **/walkthrough auto-redirects away** — NOT REPRODUCIBLE /
   DROPPED. No server-side redirect (only `/`, `/docs`, `→/trade/`
   exist), no `window.location`/meta-refresh, the only `setTimeout`
   is the logs copy-button, polls return no scripts. Agent
   misobservation. No fix made.
2. **/topology `unknown component: summary`** — FIXED. Moved the
   `/x/topology/{component}` catch-all to AFTER the literal
   `summary` route (server.py). Verified: `/x/topology/summary`
   now returns the GW/ME/MD line, 0 "unknown component".
3. **/risk actions hardcoded to user 1** — FIXED. `riskAction(action)`
   JS helper reads `#risk-uid`, templates the path, and swaps the
   result into `#risk-action-result` (also fixes the prior no-feedback
   `hx-swap="none"`). Verified: deposit→user 3, freeze→user 7.
4. **/orders silent fail when gateway down** — the KeyError-500
   (`{"detail":"1"}`) root cause was ALREADY fixed in code
   (send_order_to_gateway returns 3-tuples; secret path no longer
   a dict). Residual silent case was the 504-timeout (HTMX won't
   swap non-2xx) → FIXED with `hx-on::response-error` →
   `quickOrderError()` showing "no response (NNN) — is the gateway
   running? Control".
5. **/latency stale + leaky labels** — FIXED. "rsx-dxs bench" →
   "rsx-cast bench" (×2); removed `.ship/12-SHOWCASE-HONEST` path
   from the design-budgets text. Verified 0/0/2.

NOTE — **dashboard instability**: the playground server died 3×
during this session under concurrent browser load, each time with
NO Python traceback and NO clean shutdown in server.log (external
kill signature). Processes also flap (4/7 running). Worth a
dedicated reliability look — separate from the UX work.

## Cross-cutting themes (apply sitewide)
- **A. Jargon with zero hints.** Add `title=`/`<abbr>` tooltips
  (no prose) on: BBO, IM, MM, OI, uPnL, margin ratio, P50/P95/P99,
  ns/µs, seq, tip, lag, WAL, NAK, SPSC, cast, TIF/GTC, bps,
  leverage, "blocked", "(session)", "(simulated)", "(proxy)",
  scenario names (minimal/duo/full/stress), process codenames
  (gw-0, me-pengu, risk-0).
- **B. Process-dependency is invisible → silent failures.** Orders/
  Maker/Stress/Trade look operational but do nothing when procs/gw
  are down. Add a global status chip ("procs N/6 · gw offline →
  Control") and make dead-end actions show a visible error + a
  `→ Control` link.
- **C. Empty states with no next step.** book/wal/orders/trade tabs
  empty and silent. Add inline CTAs: "→ place a test order",
  "→ start maker", "→ watch on Book", "→ see position on Risk".
- **D. No learn-more path from any page.** Footer "Playground Docs"
  is buried; no per-section `?`/docs links. (Solved by the
  GUIDE/DOCS split + per-component doc links + section `?` links.)
- **E. Nav not graduated.** 15 equal tabs, no "start here". Bold/
  first-position Walkthrough as the entry point; consider grouping.
- **F. False / poor affordances.**
  - /verify rows hover-highlight but click does nothing → make
    clickable, expand reason inline (Verify task).
  - /logs expand = full-screen modal, undiscoverable, scroll-jumps
    → inline accordion + tail that won't clobber (Logs task).
  - /faults & /control Stop vs Kill look identical, no "what breaks
    if I kill this?" hint, no immediate feedback → tooltips +
    distinct styling + `hx-on::after-request` "stopping…".

## Per-page highlights
- **walkthrough**: redirect bug (above); sub-nav anchors don't look
  like jump-links / may not be wired; "Expand details" looks like a
  caption not a control (add ▶).
- **overview**: health score `0/100 RED` with `-60/-50/-25`
  breakdown unexplained; "GW: live", "wal 3", "errs 439", "INVARIANTS",
  "WAL STREAM LAG (PROXY)", "blocked" all need a tooltip; scenario
  names undefined.
- **topology**: summary bug (above); "CLICK A NODE" CTA too dim;
  edge labels (cast/WAL/WS) + NAK/DROP + core-affinity + "(session)"
  + "WAL tips: none" need tooltips; node detail → add "→ dedicated
  page" link (Gateway→Orders, Matching→Book, etc. = the new
  /component pages).
- **book**: empty-state needs "→ place a test order" + a "Start
  Maker" shortcut; ladder/stats/agg headers need a one-word hint.
- **risk**: column-header tooltips; fix hardcoded user buttons;
  "Measure" affordance when latency is `--`.
- **latency**: best page (has "Run one probe" + descriptions); fix
  stale/leaky labels; percentile-header tooltip; note probe "needs
  maker".
- **wal**: hardest page; one-liner under "PER-PROCESS WAL STATE";
  tooltips on lag/tip; Timeline empty-state CTA; explain Verify /
  Dump JSON result targets.
- **logs/verify/faults**: see theme F.
- **control/maker/stress/orders/trade**: see themes A–C; maker
  needs "symbol 10 = PENGU" + "→ watch on Book"; trade "Exchange
  offline" banner needs "→ Start processes" link + a "← Playground"
  back-link.

## How this maps to the build
- D (learn-more) ← GUIDE/DOCS split + per-component doc links +
  section `?` links.
- F(verify) ← Verify clickable-rows task. F(logs) ← Logs accordion.
- topology node → /component page links ← foundation.
- A/B/C/E + bugs 1–5 ← a dedicated "affordances + bugfix" pass
  (tooltips, status chip, empty-state CTAs, nav highlight). No new
  prose. Mostly `title=` attrs and small links — cheap, high value.
