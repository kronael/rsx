# The Dev Dashboard That Replaced Six Terminals

HTMX + FastAPI + zero JavaScript. 156 Playwright tests.

## The Problem

Developing an exchange means running six processes (gateway,
risk, matching engine, marketdata, recorder, mark price),
injecting orders via CLI, grepping logs across six files,
and reconstructing what happened. Kill the matching engine
to test recovery? Open another terminal. Check the orderbook
rebuilt correctly? Another terminal. Verify invariants?
Write a one-off script.

Six terminals. Copy-paste command history. Lost context
switching between windows.

## The Solution: Observe, Act, Verify

One browser tab. Everything visible. Everything controllable.

**Observe:** Process table with PIDs, uptime, status. Live
orderbook ladder per symbol. WAL sequence numbers and lag.
Ring backpressure gauges. Mark price feeds. System health
score. Error aggregation across all processes.

**Act:** Start/stop/restart/kill any process. Switch between
scenarios (minimal, full, stress). Inject test orders with
one form. Trigger liquidations. All via buttons that POST
to the API.

**Verify:** Run all ten system-wide invariants on demand.
Fills precede ORDER_DONE. Position = sum of fills. No
crossed books. SPSC preserves FIFO. Funding is zero-sum.
See violations with event details.

## Architecture: Two Python Files

```
server.py  — FastAPI routes, process management, WAL parsing
pages.py   — HTML generator (inline, no templates)
```

No Jinja2. No template directory. `pages.py` generates HTML
strings with Python f-strings. Every endpoint returns an HTML
fragment. HTMX swaps fragments into the page.

```python
@app.get("/x/processes")
async def processes_fragment():
    procs = scan_processes()
    return HTMLResponse(render_process_table(procs))
```

Browser loads shell page. HTMX attributes fetch content:

```html
<div hx-get="./x/processes"
     hx-trigger="load, every 2s"
     hx-swap="innerHTML">
  Loading...
</div>
```

On load: fetch HTML, swap in. Every 2s: fetch again, swap
again. No JavaScript. No state management. Server holds state,
server renders state.

## Ten Screens, Zero Build Step

| Screen | What it shows |
|--------|--------------|
| Overview | Process table, health score, WAL status, metrics |
| Book | Orderbook ladder, BBO strip, depth, compression |
| Orders | Submit orders, cancel, trace lifecycle |
| Risk | Positions, margin, create/delete users |
| WAL | Segment browser, record inspector, integrity |
| Verify | Run invariant checks, see violations |
| Control | Start/stop processes, switch scenarios |
| Faults | Kill/stop processes, recovery notes |
| Logs | Unified log viewer with smart search |
| Metrics | Latency histograms, throughput, ring pressure |

Each screen is a function in `pages.py` that returns HTML.
Each auto-refreshing section is a separate `/x/` endpoint.
HTMX polls the endpoint, server returns fresh HTML.

## Smart Search

The log viewer has a single search box. Type "gateway error
order" and it extracts: process=gateway, level=error,
search=order. Sets hidden filter dropdowns via JavaScript,
triggers HTMX refresh. One input replaces three dropdowns.

```html
<input id="smart-search"
       placeholder="e.g. gateway error timeout"
       onkeydown="if(event.key==='Enter') parseSmartSearch()">
```

Keyboard shortcuts: `/` focuses search, `Ctrl+L` clears all
filters. Quick filter chips for each process name.

## Confirm Gate for Destructive Actions

Buttons that kill processes or wipe state require
confirmation. CLI users send `x-confirm: yes` header.
Browser HTMX requests bypass the gate (you clicked a
button, that's confirmation enough).

```python
def check_confirm(request, endpoint):
    if endpoint not in DESTRUCTIVE_ENDPOINTS:
        return None
    if request.headers.get("hx-request"):
        return None  # HTMX = browser click = confirmed
    if request.headers.get("x-confirm") != "yes":
        return error_response("add x-confirm: yes header")
```

## 156 Playwright Tests

Every screen has a test file. Tests verify:
- Page loads without console errors
- Cards and headings render
- Auto-refresh polling works (hx-trigger attributes)
- Form submission returns expected responses
- Keyboard shortcuts work
- Filters persist across auto-refresh
- Modal dialogs open and close

11 test files, 156 tests. Run in ~30 seconds headless.

The tests caught real bugs: missing `name` attribute on an
input (HTMX `hx-include` silently sent nothing), confirm
gate blocking HTMX requests (buttons returned 400), hidden
dropdowns failing Playwright visibility checks.

## Why Not React

React makes sense for customer-facing trading UIs. Bundle
optimization, offline support, complex client state. Dev
dashboards have different needs:

- **Zero build step.** Edit Python, reload browser.
- **Server-side state.** Process table lives on server. Why
  duplicate in client state?
- **HTML fragments.** Server renders exactly what the browser
  needs. No JSON parsing, no DOM diffing.
- **Two files.** Not 200 files in node_modules.

Trade-off: no offline support, CDN dependency for Tailwind
and HTMX. Acceptable for `localhost:3000`.

## The Iteration Loop

1. Change matching engine code
2. `cargo build` (2s incremental)
3. Click "Build & Start All" in browser
4. See processes spin up in process table
5. Click "Orders" tab, submit test order
6. Click "Book" tab, see orderbook update
7. Click "Verify" tab, run invariants
8. All green. Done.

30 seconds from code change to verified correctness. No
terminal switching. No command history. No grep.

## Key Takeaways

- **HTMX eliminates client-side state** for server-rendered
  dashboards. Every refresh fetches fresh HTML.
- **Inline HTML generation** (f-strings, not templates)
  keeps the codebase to two files.
- **Playwright tests catch real bugs** that manual testing
  misses (missing attributes, header issues, visibility).
- **Confirm gates** protect destructive actions for CLI users
  while staying invisible for browser users.
- **Smart search** collapses multiple filter controls into
  one input with keyword extraction.

Two Python files. Zero JavaScript (except 20 lines for smart
search parsing). 156 tests. One browser tab replaces six
terminals.

## See Also

- `rsx-playground/server.py` — FastAPI server, all endpoints
- `rsx-playground/pages.py` — HTML generator, all screens
- `rsx-playground/tests/` — 11 Playwright test files
- `specs/2/BLOG.md` — Playground blog spec
- `blog/01-design-philosophy.md` — Overall design approach
