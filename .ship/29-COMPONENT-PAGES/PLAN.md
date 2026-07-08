# 29 — Component Pages + Docs Wiring

## SIGN-OFF (opus, 2026-05-31): REVISE → these override the body below

1. **Drop the `cast/wal` component page.** It's not a process —
   no `PROC_HINTS` key, no `_TOPO_HANDLERS` entry, no log file.
   The existing WAL tab already covers transport. Real components
   only: gateway, risk, matching, marketdata, mark, recorder,
   maker (7).
2. **Health metrics — honest reality (CPU stays out):**
   - **RSS memory: FREE** — `scan_processes()` already returns
     `mem` per process (server.py:1409/1451); show it.
   - **Latency: PARTIAL + honest-empty** — only `order_latencies`
     (risk leg), `e2e_latencies`, `gw_only_latencies` exist, via
     `/api/latency`; all are EMPTY until a probe/order runs. Page
     must render "no live samples — run a probe", never a fake 0.
     matching = bench ~340ns (link report, mark as bench);
     marketdata/mark/recorder/maker = no latency signal.
   - **Processed-msg RATE: does NOT exist** — topo `rows` are
     session snapshots/counters (labeled "(session)"), not rates.
     Maker has real counters. Show snapshots honestly-labeled; do
     NOT build rate plumbing this sprint.
   - **Queue/ring/rcvbuf depth: does NOT exist anywhere** and is
     expensive (needs Rust-side instrumentation). DROP from the
     page; mark as future. Per-component live health available
     today = **status dot + RSS + log tail + viz** (+ BBO/spread
     snapshots for matching/marketdata, real counters for maker).
   - No `rate` sparkline (no time series exists) — show the
     status/snapshot row instead.
3. **Docs nav: use the NEW-TAB fallback.** Rendering docs inside
   `layout()` means duplicating the marked.js + highlight.js +
   `#content` CSS stack (server.py:2818-2945) — over-engineering.
   Keep the standalone docs shell; just generalize its root
   allowlist + GUIDE/DOCS sidebar. Docs open in a new tab
   (`target="_blank"`, already used) so the playground top bar is
   never lost (it stays in the original tab).
4. **Per-component pass = serialized, registry-data-only.**
   Foundation lands the FULL `COMPONENTS` dict (all 7 stubbed) +
   generic `component_page()` template + `/component/{key}` route,
   committed green. Then per-component agents are READ-ONLY: each
   boots the running system, curls `/component/{key}`, returns its
   dict-entry values (blurb, verified doc paths, viz choice) as
   text; MAIN applies them one at a time to the single dict and
   commits after each. NO parallel writers to pages.py. No
   per-component module files (over-engineering).
5. **Logs rewrite coupling:** the modal functions are
   `showFullLine(el, i)` (pages.py:1585, wired from `render_logs`
   `onclick` at 3151) and `closeModal()` (1592) — NOT
   `openLogModal`. `render_logs` (3132) and the modal/JS (1512-
   1594) must change together or the onclick references a deleted
   fn.
6. **Don't forget:** add a "Components" landing tab to `TABS`
   (index of the 7) so component pages are reachable from nav;
   decide `active_tab` highlight for `/component/*` (lit Topology
   or none); BEFORE deleting the modal, grep Playwright/API tests
   for `#log-modal`/`showFullLine`/`closeModal` and update them;
   design the latency count==0 empty state. Gates: new routes need
   gate-2/gate-3 smoke + ≥1 Playwright (component page renders,
   inline-expand works, modal gone). Release gate = 421/421.

---

# 29 — Component Pages + Docs Wiring

Goal: a newcomer opening the playground learns each component
by seeing, per component: what it is + does, a link to its
docs, its live status, a log tail from the running system,
latency indicators, and a simple visualization (book viz for
the orderbook). Everything simple. Plus: wire the full platform
docs into the playground and split GUIDE (how to use the
playground) from DOCS (what the platform is).

## Three workstreams

### A. OAuth 404 (in progress, separate agent)
Root cause: nginx has no `/auth` / `/oauth` location → nginx
404. Fix = add those locations → playground :49171 (already
proxies to rsx-auth). Secondary blocker: empty GitHub creds in
`.env` (flagged, not fabricated). Owned by the background
sonnet agent. Not part of the oracle sign-off.

### B. Docs: GUIDE vs DOCS
- **GUIDE** = `rsx-playground/docs/` (README, api, scenarios,
  tabs, troubleshooting, getting-started/). How to *use* the
  playground. Unchanged content; relabel as GUIDE.
- **DOCS** = the platform itself. Sources already in repo:
  - `docs/concepts/*.md` (10 concept explainers + glossary)
  - `docs/demo.md`, `docs/benches.md`
  - selected `specs/2/*.md` (the authoritative spec)
- Implementation: extend the existing `/docs/{filename:path}`
  route (server.py:2757). Today it resolves only under
  `rsx-playground/docs` with a prefix-containment safety check.
  Generalize to an ALLOWLIST of roots:
  `{guide: rsx-playground/docs, concepts: docs/concepts,
   docs: docs, spec: specs/2}`. Path key prefixes the filename
  (`/docs/spec/11-gateway`, `/docs/concepts/tiles-and-pinning`,
  `/docs/guide/api`). Keep the traversal guard per-root.
- Sidebar grows two labelled groups: **GUIDE** (playground
  pages) and **DOCS** (Concepts → key Specs). Active-link
  highlight preserved.
- The `Docs` nav tab stays; default landing = a short index
  that explains the GUIDE/DOCS split.
- **Nav behavior (owner):** opening a DOCS/GUIDE page should NOT
  lose the playground top bar — preferred: render docs INSIDE the
  standard `layout()` wrapper (same top nav) instead of the
  current standalone docs HTML shell, so the nav persists while
  reading. If keeping the top bar in-page is hard, the acceptable
  fallback is to open docs in a NEW window/tab (the Docs tab link
  already uses `target="_blank"`). Pick in-layout if cheap; else
  new-tab. The platform DOCS (long specs) is the one most OK to
  open in a new window if needed.

### C. Per-component pages (the core ask)
One shared, DRY template — NOT 8 bespoke pages.

`COMPONENTS` registry (in pages.py), one entry per component:

| key        | name       | log_key    | latency block | viz   |
|------------|------------|------------|---------------|-------|
| gateway    | Gateway    | gateway    | gw_only       | rate  |
| risk       | Risk       | risk       | e2e (risk leg)| rate  |
| matching   | Matching   | matching   | me match (ns) | book  |
| marketdata | Marketdata | marketdata | —             | bbo   |
| mark       | Mark       | mark       | —             | rate  |
| recorder   | Recorder   | recorder   | —             | rate  |
| cast/wal   | Transport  | (all)      | —             | wal   |
| maker      | Maker      | maker      | —             | rate  |

Each registry entry carries:
- `blurb`: 2–3 sentence "what it is / what it does" (sourced
  from each crate's ARCHITECTURE.md + CLAUDE.md, condensed).
- `docs`: list of (label, viewer-path) into workstream B —
  e.g. gateway → `spec/11-gateway`, `concepts/tiles-and-pinning`.
- `guide`: (label, viewer-path) into the GUIDE.
- `log_key`: passed to `read_logs(process=...)` (reuses
  PROC_HINTS — already maps gateway→[gw-…] etc.).
- `topo_key`: reuse `_TOPO_HANDLERS[key]()` for status/pid/
  uptime/rows.
- `latency`: which `/api/latency` block to surface, or a note
  when only synthetic/bench numbers exist (be honest — don't
  invent per-component p50s we don't measure).
- `viz`: `book` (reuse `/x/book`), `bbo`, `wal` (reuse
  `/x/wal-status`), or `rate` (sparkline from topology flow
  rate) — all reuse existing endpoints.

New route `GET /component/{key}` → `pages.component_page(key)`:
1. Header: name + live status dot (htmx poll `/x/topology/{key}`).
2. "What it is" blurb + doc links (DOCS) + guide link.
3. Live: log tail (htmx poll, `read_logs(log_key)`), health
   metrics row (see below), latency indicator, viz panel.
All htmx polling, all reusing existing `/x/*` endpoints. No new
data plumbing unless a gap is found (then log the gap, don't
fake numbers).

### Health metrics — NOT CPU (owner constraint)
Busy-spin tiles (risk hot loop, etc.) legitimately peg a core at
100% CPU by design — CPU% is a MISLEADING health signal and must
not be shown as a load/alarm gauge. A spinning component at 100%
CPU is GREEN/healthy. Per-component health surfaces instead:
- **status dot green** when the process is up and keeping up.
- **queue / ring depths** (SPSC backpressure, UDP rcvbuf, persist
  ring) — the real "is it falling behind" signal.
- **processed-message counts / rates** (orders, fills, casts,
  WAL records) — throughput, not utilization.
- **memory** — shown and WATCHED for every component (RSS), kept
  in check; a climbing RSS is the thing to flag, not CPU.
If a depth/rate/RSS isn't already exposed by an `/x/*` endpoint or
the topology handler rows, log the gap; add a minimal reader only
where cheap. Never invent numbers.

**Live timings on the resource view.** Alongside memory/queues/
throughput, the resource-usage panel shows LIVE timings, not just
static bench numbers — refreshed via htmx poll from the existing
live latency probe (`/api/latency` e2e + gw_only blocks,
`/api/latency-probe`). Label honestly which leg each number
measures (e2e GW→ME→GW vs gw-only); where only a synthetic/bench
figure exists (e.g. ME ~340ns match), mark it as bench and link
the report rather than dressing it as a live per-component p50.

### D. Logs view (separate review agent → implement here)
Owner wants the Logs view professional-grade through simplicity:
a lean, more-ergonomic Kibana-style explorer, "no fluff". Hard
requirement: **expand INLINE (accordion row), not the current
full-screen modal** (`log-modal`, pages.py ~1512 + openLogModal/
closeLogModal JS ~1586 — delete it). Keep htmx+Tailwind+vanilla
JS. Live tail must not yank scroll or close an expanded row.
A read-only sonnet review agent produces the redesign spec;
implemented in this sprint after sign-off. Same component-page
log-tail panel should reuse the redesigned inline-expand pattern.

Wiring: each topology node links to its `/component/{key}`;
the Book/Risk/WAL/Latency tabs cross-link to the matching
component page.

## Implementation order (after sign-off)
1. Foundation (one change): docs ALLOWLIST + GUIDE/DOCS sidebar
   (B), `COMPONENTS` registry + `component_page()` template +
   `/component/{key}` route (C skeleton, all entries stubbed).
   Build + gate-1/2/3 green.
2. Per-component pass: one sonnet agent per component, each
   verifies its page against the *running* system (real log
   tail, real status, viz renders), tightens the blurb, fixes
   doc links. To avoid write conflicts on shared files, agents
   return patches/blurbs; main applies sequentially — OR each
   runs in worktree isolation. Agents MUST show real rendered
   evidence (curl/screenshot), not claims.
3. Verify: `make gate` (1-3) + Playwright additions for the new
   routes; update TABS/tests; diary entry.

## Non-goals / guardrails
- No fabricated latency numbers. If we don't measure a
  component's latency, say so and link the bench/spec instead.
- No new heavy frameworks — htmx + Tailwind, matching existing
  pages. "Everything simple."
- Trust boundaries unchanged (cast stays unauthenticated, etc.).
- No external publishing. No git push.
