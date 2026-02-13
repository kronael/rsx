# Building a Playground for a Perpetuals Exchange

Traditional exchange development is blind. You compile, launch six
processes, inject test orders, and hope for correctness. When
something breaks, you're left grep'ing logs across multiple files,
reconstructing timelines manually, and guessing which process
violated an invariant. You need visibility into every metric, the
ability to trigger failures on demand, and a way to verify
correctness interactively. The RSX Playground provides this:
observe everything, control everything, verify everything.

## The Three Pillars: Observe, Act, Verify

The playground is organized around three capabilities.
**Observe** means seeing every metric and internal state: process
health, orderbook ladders, WAL sequence numbers, CMP packet loss,
margin ratios, SPSC ring backpressure, mark price staleness. No
metric is hidden. **Act** means controlling the system:
start/stop/restart processes, inject orders, trigger liquidations,
pause components mid-operation, corrupt WAL files, partition
networks, reset state. **Verify** means checking invariants: fills
precede ORDER_DONE, positions equal sum of fills, no crossed books,
SPSC preserves FIFO order, funding payments are zero-sum. Every
correctness rule becomes a checkable assertion.

Traditional exchanges provide production monitoring dashboards but
no interactive dev tooling. You observe metrics in isolation,
trigger actions via command-line scripts scattered across repos,
and verify correctness by eyeballing logs. The playground unifies
all three capabilities in a single interface. See the problem,
trigger the action, verify the result, all within seconds.

## Architecture

The playground wraps RSX's existing process management script
(Python-based `start`) with an HTTP API layer. A FastAPI server
exposes 22 REST endpoints for control actions (POST
/api/processes/{name}/start, POST /api/processes/{name}/kill) and
a Server-Sent Events endpoint for live metrics (GET /api/events).
The server runs on port 3000 alongside a static HTML dashboard
that polls REST endpoints and subscribes to SSE streams.

Two usage modes: CLI mode sends one-shot requests (curl the API,
get JSON back), dashboard mode renders everything visually in the
browser. No authentication, no TLS. This is local dev only. The API
wraps subprocess management, exposes process stdout/stderr as log
streams, and reads metrics from each component's structured log
output. Components already emit JSON-formatted logs with fields
like `seq`, `ts_ns`, `latency_us`, `ring_full_pct`. The playground
just parses and aggregates them.

No agent persistence, no multi-user state. Start the playground,
launch a scenario, observe, act, verify. Stop the playground, all
state disappears. The underlying RSX processes persist their own
state (WAL files, Postgres, tmp directory). The playground is
stateless orchestration.

## Key Screens Walkthrough

**Overview** (Screen 1): System health at a glance. Process
table showing PID, uptime, CPU, memory, pinned cores. Health score
(0-100) aggregating all metrics. Alerts for crossed books, sequence
gaps, excessive latency. See immediately whether the system is
healthy or degraded.

**Book** (Screen 3): Live orderbook ladder for each symbol. Ten
levels per side, price/size/order count columns, updating every
100ms. BBO strip showing all symbols in a grid. Book depth chart
with cumulative size visualization. Compression map utilization
and recentering count.

**Control** (Screen 7): Process management panel.
Start/stop/restart/kill buttons per process. Launch predefined
scenarios (minimal, full, stress, replication). Clean state (wipe
tmp, reset database, delete WAL files). View process dependency
graph.

**Faults** (Screen 8): Simple fault injection via process
kill/stop endpoints. Kill a process to simulate crashes, stop
to simulate hangs. Network-level faults use OS tools (iptables,
tc) directly. Recovery observable in Overview and Book screens.

**Verify** (Screen 9): Invariant checking dashboard. Run all
ten system-wide correctness rules on demand (GET
/api/verify/invariants). Display violations with event details:
which order violated "fills before ORDER_DONE", which sequence
number violated monotonicity, which user has position != sum of
fills. Reconciliation checks (frozen margin vs computed, shadow
book vs ME book, mark price vs index).

## Failure Injection and Recovery Testing

Exchange correctness depends on recovery behavior. The Faults
screen (Screen 8) makes this testable. Kill the matching engine
mid-operation (POST /api/processes/matching-BTCUSD/kill). Watch
the WAL replay from the last persisted tip. Verify the rebuilt
orderbook matches the pre-crash state via invariant checks (GET
/api/verify/invariants). Check that all in-flight orders resume
processing without duplication. Kill risk entirely. Watch the
gateway reject new orders with "risk unavailable". Restart risk,
verify queued orders flush through.

For network-level faults, use OS tools directly: iptables rules
between gateway and risk to simulate partitions, tc for latency
injection. Watch the NACK count climb in the metrics, retransmits
trigger, and orders eventually deliver.

## How It Accelerates Development

Instead of printf debugging across six processes, see everything
in one dashboard. Live orderbook (Screen 3) shows whether your
modify operation correctly updated the price level. Fill feed
shows per-stage latency annotations (GW recv -> Risk validated ->
ME matched -> Fill emitted). SPSC ring backpressure gauge shows
whether your producer is stalling the consumer. WAL lag dashboard
(Screen 5) shows whether your consumer is falling behind.

Run scenarios via the start script (`./start stress`). Testing a
liquidation? Launch stress mode, inject orders via the API (POST
/api/orders/test), watch the liquidation queue populate (GET
/api/risk/liquidations), verify the position closes at mark price,
check funding payments sum to zero via invariant checks.

Catch invariant violations before they reach production. The
Verify screen (Screen 9, GET /api/verify/invariants) runs all ten
correctness checks on demand. "Fills before ORDER_DONE" scans
every order in the WAL, verifies FILL events precede ORDER_DONE in
sequence order, fails with event details if violated. "Position =
sum of fills" recomputes positions from fill history, compares to
risk engine state, reports discrepancies. "Slab no-leak" checks
the orderbook allocator, ensures allocated slots equal free list
plus active orders.

Development velocity increases because the feedback loop
tightens. Change matching engine logic, rebuild, restart ME (POST
/api/processes/matching-BTCUSD/restart), inject orders (POST
/api/orders/test), see the book update (Screen 3), verify
invariants (Screen 9), all within 30 seconds. No manual log
parsing. No multi-terminal orchestration. No blind hope.

## Why HTMX for Dev Dashboards

Production exchanges use React or Vue for their dashboards: npm
install, webpack config, JSX transpilation, hot module reload,
state management libraries, 50MB node_modules. Five-minute build
times. Bundle size optimization. This complexity makes sense for
customer-facing apps where offline support, bundle size, and UX
polish matter. Dev dashboards have different priorities: fast
iteration, zero build step, minimal dependencies. HTMX delivers
this.

HTMX extends HTML with attributes like `hx-get`, `hx-post`,
`hx-trigger`, `hx-swap`. A button with `hx-post="/api/restart"`
`hx-target="#status"` sends POST on click, swaps returned HTML
into #status element. No JavaScript written. No build step. Edit
HTML template, refresh browser, see changes. Server returns HTML
fragments, not JSON. The API endpoint renders the same template
the full page uses.

The entire RSX playground frontend is two Python files: app.py
(FastAPI routes) and templates/index.html (single-page layout with
HTMX). Zero JavaScript. No package.json. No build artifacts. Start
the server, open localhost:3000, everything works. Change a
template, reload the page, see updates. Iteration time measured in
seconds.

## RSX Playground Architecture

FastAPI backend with Jinja2 templates. Each API endpoint returns
either JSON (for CLI curl usage) or HTML (for browser requests).
Content negotiation via Accept header. Request JSON, get JSON.
Request HTML, get rendered template fragment.

```python
@app.get("/api/processes")
async def get_processes(request: Request):
    procs = await process_manager.list_all()
    if "text/html" in request.headers.get("accept", ""):
        return templates.TemplateResponse(
            "partials/process_table.html",
            {"request": request, "processes": procs}
        )
    return {"processes": procs}
```

Browser requests carry `Accept: text/html`, get back a table row
fragment. CLI curl defaults to JSON. One endpoint, two consumers,
no duplication.

Tailwind Play CDN for styling. Include script tag in <head>, use
utility classes directly. No CSS compilation. No purge step. No
watching file changes. JIT compilation happens in-browser on first
load (500ms delay), then cached. Dark theme via `dark:` prefix on
every class.

```html
<script src="https://cdn.tailwindcss.com?plugins=forms"></script>
<script>
  tailwind.config = {
    darkMode: 'class',
    theme: { extend: {} }
  }
</script>
```

Note: Tailwind Play uses a script tag, not a CSS link. The Play
CDN doesn't serve pre-built CSS files. Script tag loads the JIT
engine, scans document for classes, generates CSS dynamically.

## Pattern: HTML Fragments and HTMX Swaps

Full page load renders the shell: navigation, containers with IDs,
HTMX script tag. Content areas marked with `hx-get` and
`hx-trigger="load"` fetch their data on page load. Subsequent
updates use polling or manual triggers.

```html
<div id="process-list"
     hx-get="/api/processes"
     hx-trigger="load, every 2s"
     hx-swap="innerHTML">
  <p class="text-gray-500">Loading processes...</p>
</div>
```

On page load, HTMX fetches /api/processes, expects HTML back,
swaps into #process-list via innerHTML. Then polls every 2s.
Server endpoint renders process_table.html template, returns raw
HTML. No JSON parsing. No DOM manipulation code. Declarative.

Action buttons use hx-post:

```html
<button hx-post="/api/processes/gateway/restart"
        hx-target="#process-list"
        hx-swap="outerHTML"
        class="px-3 py-1 bg-blue-600 text-white rounded">
  Restart
</button>
```

Click button, POST to endpoint, endpoint restarts process and
returns updated process table HTML, HTMX swaps entire #process-list
container. User sees immediate feedback. No client-side state
update logic.

## Polling for Live Data

Orderbook updates every 100ms. Mark price every 1s. Process health
every 2s. HTMX handles polling via `hx-trigger="every Ns"`.

```html
<div id="orderbook"
     hx-get="/api/book/BTCUSD"
     hx-trigger="every 100ms"
     hx-swap="innerHTML">
</div>
```

Server renders book ladder template with bid/ask levels, price,
size, order count. Returns HTML table. HTMX swaps it in. Next tick
fetches again. Network overhead minimal (1-2KB per response).
Server CPU minimal (template render <1ms). Good enough for dev
dashboards.

Alternative: Server-Sent Events for push-based updates. HTMX
supports `hx-trigger="sse:eventname"` but requires htmx-sse
extension. Polling simpler, fewer moving parts.

## Dark Theme with Tailwind Play CDN

Tailwind dark mode via `dark:` class prefix. Set `dark` class on
<html> element, all `dark:bg-gray-900` rules activate.

```html
<html lang="en" class="dark">
<body class="bg-white dark:bg-gray-900 text-black dark:text-white">
  <div class="border border-gray-300 dark:border-gray-700">
    <button class="bg-blue-500 hover:bg-blue-600 dark:bg-blue-700
                   dark:hover:bg-blue-800">
      Action
    </button>
  </div>
</body>
</html>
```

Every element needs explicit dark variant. Verbose but predictable.
Add toggle button with Alpine.js (3KB) or vanilla JS one-liner:

```html
<button onclick="document.documentElement.classList.toggle('dark')">
  Toggle Theme
</button>
```

Persist preference via localStorage if needed. Ten lines of
JavaScript total.

## Action Buttons with hx-post

Process control panel: start, stop, restart, kill buttons per
process. Each button hits different endpoint, returns updated
status.

```html
<div id="control-{{process.name}}">
  <button hx-post="/api/processes/{{process.name}}/start"
          hx-target="#control-{{process.name}}"
          hx-swap="outerHTML"
          hx-disabled-elt="this"
          class="px-2 py-1 bg-green-600 text-white rounded">
    Start
  </button>
  <button hx-post="/api/processes/{{process.name}}/kill"
          hx-target="#control-{{process.name}}"
          hx-swap="outerHTML"
          hx-disabled-elt="this"
          class="px-2 py-1 bg-red-600 text-white rounded">
    Kill
  </button>
</div>
```

`hx-disabled-elt="this"` disables button during request (prevents
double-click). Endpoint executes action, renders updated button
group (maybe "Stop" button now enabled, "Start" disabled), returns
HTML. HTMX swaps entire control div. User sees button states
change based on process state.

## Forms that Return HTML Responses

Order injection form: user fills symbol, side, price, quantity,
hits submit. Endpoint validates, injects order via CMP, returns
confirmation or error HTML.

```html
<form hx-post="/api/orders/inject"
      hx-target="#order-result"
      hx-swap="innerHTML">
  <input name="symbol" type="text" placeholder="BTCUSD"
         class="border dark:border-gray-700 rounded px-2 py-1">
  <select name="side">
    <option value="buy">Buy</option>
    <option value="sell">Sell</option>
  </select>
  <input name="price" type="number" step="0.01">
  <input name="qty" type="number" step="0.001">
  <button type="submit"
          class="px-4 py-2 bg-blue-600 text-white rounded">
    Inject Order
  </button>
</form>
<div id="order-result"></div>
```

Backend route:

```python
@app.post("/api/orders/inject")
async def inject_order(request: Request, symbol: str = Form(...),
                       side: str = Form(...), price: float = Form(...),
                       qty: float = Form(...)):
    try:
        oid = await order_injector.send(symbol, side, price, qty)
        return templates.TemplateResponse(
            "partials/order_success.html",
            {"request": request, "oid": oid}
        )
    except ValueError as e:
        return templates.TemplateResponse(
            "partials/order_error.html",
            {"request": request, "error": str(e)},
            status_code=400
        )
```

Success template:

```html
<div class="p-4 bg-green-900 border border-green-700 rounded">
  Order injected: {{oid}}
</div>
```

Error template:

```html
<div class="p-4 bg-red-900 border border-red-700 rounded">
  Error: {{error}}
</div>
```

HTMX submits form as standard POST with
application/x-www-form-urlencoded, gets HTML back, swaps into
#order-result. No JSON. No client-side validation framework. Server
validates, renders appropriate template, returns HTML. Browser
displays result.

## No JavaScript Written

Entire playground frontend: zero .js files. All interactivity via
HTMX attributes. Toggle theme? One-line onclick. Need client-side
logic? Use Alpine.js (declarative, attribute-based like HTMX, 3KB).
RSX playground doesn't need it yet.

Compare to React equivalent: useState, useEffect for polling,
fetch() in every component, JSON parsing, state updates, re-renders,
prop drilling, context providers for shared state. HTMX collapses
this to HTML attributes. Server holds state, server renders state,
client just swaps HTML.

Developer experience: edit Python route, edit Jinja2 template,
reload browser. See changes. No transpilation. No build watch
mode. No module resolution. No bundler errors. Fast feedback loop.

## Trade-offs

CDN dependency: Tailwind Play and HTMX both load from CDN. No
internet, no styles or interactivity. Dev dashboards typically run
on local network with internet access, so acceptable. Production
dashboards should self-host everything.

JIT compilation delay: Tailwind Play scans DOM for classes on first
load, generates CSS. 500ms blank screen on slow connections.
Subsequent loads cached. Self-hosted Tailwind with purge step
eliminates this but requires build step.

Server-side rendering load: Every poll generates template render on
server. FastAPI + Jinja2 handle ~1ms per render. At 10 dashboards
polling 10 endpoints every 2s, that's 50 renders/sec = 50ms CPU.
Negligible. Scales to ~100 concurrent users before templating
becomes bottleneck. Dev dashboards serve 1-5 users.

No offline support: Dashboard requires server running. React apps
can bundle static assets, work offline with cached data. Dev
dashboards depend on live system state anyway, so offline mode
irrelevant.

SEO and initial load: Server-rendered HTML loads instantly. HTMX
fetches content after page load, so "Loading..." flicker. For dev
tools, irrelevant. Public-facing sites should pre-render critical
content.

## Conclusion

Two Python files: app.py (API routes + template rendering),
templates/index.html (layout + HTMX attributes). Zero JavaScript.
Zero build step. Full-featured dashboard with live polling, action
buttons, form handling, dark theme. Start server, open browser,
see data. Edit template, reload page, see changes. Iteration time
measured in seconds.

HTMX and Tailwind Play CDN eliminate complexity for projects where
build tooling overhead exceeds benefit. Dev dashboards, admin
panels, internal tools, prototypes all fit this profile. Customer-
facing apps with offline requirements, bundle size constraints, or
complex client-side state management still favor React/Vue. Choose
the right tool for the job. For RSX Playground, HTMX is the right
tool.
