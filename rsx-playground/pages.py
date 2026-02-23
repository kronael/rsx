"""Inline HTML generation for RSX Playground dashboard.

Uses Tailwind Play CDN (script tag, JIT compiler in browser)
+ HTMX for interactivity. All URLs relative for proxy compat.
"""

import html

TABS = [
    ("Overview", "./overview"),
    ("Topology", "./topology"),
    ("Book", "./book"),
    ("Risk", "./risk"),
    ("WAL", "./wal"),
    ("Logs", "./logs"),
    ("Control", "./control"),
    ("Maker", "./maker"),
    ("Faults", "./faults"),
    ("Verify", "./verify"),
    ("Orders", "./orders"),
    ("Stress", "./stress"),
    ("Docs", "./docs"),
    ("Trade", "./trade/"),
]


def layout(title, content, active_tab="./overview"):
    """Wrap content in full page with nav tabs."""
    tabs = ""
    for tab_item in TABS:
        if len(tab_item) == 3:
            label, href, external = tab_item
        else:
            label, href = tab_item
            external = False

        if href == active_tab:
            cls = ("bg-slate-700 text-white "
                   "px-3 py-1.5 rounded text-xs font-mono")
        else:
            cls = ("text-slate-400 hover:text-white "
                   "hover:bg-slate-700 px-3 py-1.5 "
                   "rounded text-xs font-mono")

        target = ' target="_blank" rel="noopener noreferrer"' if external else ''
        tabs += f'<a href="{href}"{target} class="{cls}">{label}</a>\n'

    return f"""<!DOCTYPE html>
<html lang="en" class="dark">
<head>
<meta charset="utf-8">
<meta name="viewport"
  content="width=device-width, initial-scale=1">
<title>RSX -- {title}</title>
<script src="https://cdn.tailwindcss.com"></script>
<script src="./static/htmx.min.js"></script>
<script>
tailwind.config = {{
  darkMode: 'class',
  theme: {{
    extend: {{
      fontFamily: {{
        mono: ['SF Mono', 'Cascadia Code', 'Fira Code',
          'ui-monospace', 'monospace'],
      }},
    }},
  }},
}}
</script>
<style type="text/tailwindcss">
  body {{ font-family: theme('fontFamily.mono'); }}
  .htmx-indicator {{ display: none; }}
  .htmx-request .htmx-indicator {{ display: inline; }}
  .htmx-request.htmx-indicator {{ display: inline; }}
</style>
<script>
document.addEventListener('htmx:afterSwap', function(e) {{
  var t = e.detail.target;
  if (t.id === 'log-view' || t.classList.contains('logs-auto-scroll')) {{
    t.scrollTop = t.scrollHeight;
  }}
}});
</script>
</head>
<body class="bg-slate-950 text-slate-300 min-h-screen
  text-[13px]">
<nav class="flex flex-wrap items-center gap-1 px-2
  sm:px-4 py-2 bg-slate-900 border-b border-slate-800">
  <span class="text-sm font-bold text-blue-400
    mr-4 tracking-wider">RSX</span>
  {tabs}
</nav>
<main class="p-2 sm:p-4 max-w-7xl mx-auto space-y-3">
{content}
</main>
<footer class="mt-8 py-4 px-4 border-t border-slate-800
  bg-slate-900 text-center">
  <div class="max-w-7xl mx-auto flex flex-wrap justify-center
    items-center gap-4 text-xs text-slate-500">
    <span>RSX Playground</span>
    <span class="text-slate-700">|</span>
    <a href="./docs" target="_blank"
      class="text-blue-400 hover:text-blue-300">
      Playground Docs</a>
    <span class="text-slate-700">|</span>
    <a href="./trade/"
      class="text-blue-400 hover:text-blue-300">
      Trade UI</a>
  </div>
</footer>
</body>
</html>"""


def _card(title, body, header_right=""):
    """Helper: card with title and optional header actions."""
    right = ""
    if header_right:
        right = f'<div class="flex gap-2">{header_right}</div>'
    return f"""
<div class="bg-slate-900 border border-slate-800
  rounded-lg p-4">
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-xs font-semibold text-slate-500
      uppercase tracking-wider">{title}</h2>
    {right}
  </div>
  {body}
</div>"""


# ── Screen 1: Overview ──────────────────────────────────

def overview_page():
    welcome = """<div class="bg-slate-900 border border-slate-800
  rounded-lg p-3 flex items-center justify-between">
  <div>
    <span class="text-xs text-slate-400">
      RSX Playground - Development Dashboard</span>
  </div>
  <div class="flex gap-3 text-xs">
    <a href="./docs/api"
      class="text-blue-400 hover:text-blue-300">
      API Reference</a>
  </div>
</div>"""
    health = _card(
        "System Health",
        '<div hx-get="./x/health" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    procs = _card(
        "Process Table",
        '<div hx-get="./x/processes" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
        header_right=(
            '<div class="flex flex-wrap gap-2 items-center">'
            '<div class="flex flex-wrap gap-1" role="group">'
            '<label class="cursor-pointer">'
            '<input type="radio" name="scenario-ov" '
            'id="scenario" value="minimal" '
            'class="sr-only peer" checked>'
            '<span class="peer-checked:bg-zinc-500 '
            'peer-checked:text-white bg-zinc-700 '
            'text-zinc-300 px-2 py-1 rounded text-xs '
            'font-medium hover:bg-zinc-600 '
            'transition-colors block">minimal</span>'
            '</label>'
            '<label class="cursor-pointer">'
            '<input type="radio" name="scenario-ov" '
            'value="duo" class="sr-only peer">'
            '<span class="peer-checked:bg-zinc-500 '
            'peer-checked:text-white bg-zinc-700 '
            'text-zinc-300 px-2 py-1 rounded text-xs '
            'font-medium hover:bg-zinc-600 '
            'transition-colors block">duo</span>'
            '</label>'
            '<label class="cursor-pointer">'
            '<input type="radio" name="scenario-ov" '
            'value="full" class="sr-only peer">'
            '<span class="peer-checked:bg-zinc-500 '
            'peer-checked:text-white bg-zinc-700 '
            'text-zinc-300 px-2 py-1 rounded text-xs '
            'font-medium hover:bg-zinc-600 '
            'transition-colors block">full</span>'
            '</label>'
            '<label class="cursor-pointer">'
            '<input type="radio" name="scenario-ov" '
            'value="stress" class="sr-only peer">'
            '<span class="peer-checked:bg-amber-600 '
            'peer-checked:text-white bg-zinc-700 '
            'text-zinc-300 px-2 py-1 rounded text-xs '
            'font-medium hover:bg-zinc-600 '
            'transition-colors block">stress</span>'
            '</label>'
            '</div>'
            '<script>'
            'document.querySelectorAll('
            '\'input[name="scenario-ov"]\')'
            '.forEach(function(r) {'
            'r.addEventListener(\'change\', function() {'
            'document.getElementById(\'scenario\')'
            '.value = this.value; });});'
            '</script>'
            '<button class="bg-emerald-900/60 text-emerald-400 '
            'px-3 py-1 rounded text-xs border '
            'border-emerald-800 hover:bg-emerald-800 '
            'cursor-pointer" '
            'hx-post="./api/processes/all/start" '
            'hx-vals="js:{scenario: '
            'document.getElementById(\'scenario\').value}" '
            'hx-target="#start-result" '
            'hx-swap="innerHTML" '
            'hx-indicator="#build-spin">'
            'Build &amp; Start All</button>'
            '<button class="bg-red-900/40 text-red-400 '
            'px-3 py-1 rounded text-xs border '
            'border-red-900 hover:bg-red-900 '
            'cursor-pointer" '
            'hx-post="./api/processes/all/stop" '
            'hx-target="#start-result" '
            'hx-swap="innerHTML">Stop All</button>'
            '<span id="build-spin" class="htmx-indicator '
            'text-blue-400 text-xs animate-pulse">'
            'building...</span>'
            '<span id="start-result" class="text-xs">'
            '</span>'
            '</div>'
        ),
    )
    metrics = _card(
        "Key Metrics",
        '<div hx-get="./x/key-metrics" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    rings = _card(
        "Ring Backpressure",
        '<div hx-get="./x/ring-pressure" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    invariants = _card(
        "Invariants",
        '<div hx-get="./x/invariant-status" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    wal_status = _card(
        "WAL Status",
        '<div hx-get="./x/wal-status" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    logs_tail = _card(
        "Logs (tail)",
        '<div class="max-h-40 overflow-y-auto logs-auto-scroll" '
        'hx-get="./x/logs-tail" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    stats = _card(
        "Stats",
        '<div hx-get="./x/stats" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    pulse = (
        '<div class="bg-slate-900 border border-slate-800 '
        'rounded-lg px-4 py-2 flex flex-wrap items-center '
        'gap-4 sm:gap-6 text-xs font-mono" '
        'hx-get="./x/pulse" '
        'hx-trigger="load, every 1s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>'
    )
    content = f"""
{welcome}
{pulse}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{health}
{procs}
</div>
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{metrics}
{stats}
</div>
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{rings}
{wal_status}
</div>
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{logs_tail}
{invariants}
</div>"""
    return layout("Overview", content, "./overview")


# ── Screen 2: Topology ──────────────────────────────────


def render_topology_node(name, key, status, rate_label):
    """Single component box with status dot and rate label."""
    dot = (
        "bg-emerald-400" if status == "running"
        else "bg-red-500" if status == "stopped"
        else "bg-zinc-600"
    )
    return (
        f'<button hx-get="./x/topology/{key}" '
        f'hx-target="#component-detail" hx-swap="innerHTML" '
        f'id="topo-node-{key}" '
        f'class="component-node bg-zinc-800 border-2 '
        f'border-zinc-600 rounded-lg p-3 '
        f'hover:border-emerald-500 cursor-pointer text-left '
        f'min-w-[110px] transition-colors">'
        f'<div class="flex items-center gap-2 mb-1">'
        f'<span id="topo-dot-{key}" '
        f'class="w-2 h-2 rounded-full {dot} shrink-0"></span>'
        f'<span class="text-xs font-mono font-bold '
        f'text-slate-200">{html.escape(name)}</span>'
        f"</div>"
        f'<div id="topo-rate-{key}" '
        f'class="text-xs text-zinc-400 font-mono">'
        f"{html.escape(rate_label)}</div>"
        f"</button>"
    )


def render_component_detail(component, data):
    """Detail card for a topology component."""
    name = data.get("name", component)
    status = data.get("status", "unknown")
    pid = data.get("pid", "-")
    uptime = data.get("uptime", "-")
    rows = data.get("rows", [])

    dot = (
        "bg-emerald-400" if status == "running"
        else "bg-red-500" if status == "stopped"
        else "bg-zinc-600"
    )

    rows_html = "".join(
        f'<div class="flex justify-between gap-4 py-0.5 '
        f'border-b border-zinc-800 last:border-0">'
        f'<span class="text-zinc-500">'
        f"{html.escape(str(k))}</span>"
        f'<span class="text-zinc-200 font-mono">'
        f"{html.escape(str(v))}</span>"
        f"</div>"
        for k, v in rows
    )

    return (
        f'<div class="bg-zinc-900 border border-zinc-700 '
        f'rounded-lg p-4">'
        f'<div class="flex items-center gap-3 mb-3">'
        f'<span class="w-2.5 h-2.5 rounded-full {dot}"></span>'
        f'<span class="text-sm font-mono font-bold '
        f'text-slate-200">{html.escape(name)}</span>'
        f'<span class="text-xs text-zinc-500 ml-1">'
        f"{html.escape(status)}</span>"
        f'<span class="ml-auto text-xs text-zinc-500 font-mono">'
        f"pid: {html.escape(str(pid))}"
        f"&nbsp;&nbsp;uptime: {html.escape(str(uptime))}"
        f"</span>"
        f"</div>"
        f'<div class="text-xs">{rows_html}</div>'
        f"</div>"
    )


def topology_page():
    client = render_topology_node(
        "Clients", "client", "unknown", "WS"
    )
    gateway = render_topology_node(
        "Gateway", "gateway", "unknown", "\u2014"
    )
    risk = render_topology_node(
        "Risk", "risk", "unknown", "\u2014"
    )
    matching = render_topology_node(
        "Matching", "matching", "unknown", "\u2014"
    )
    marketdata = render_topology_node(
        "Marketdata", "marketdata", "unknown", "\u2014"
    )
    mark = render_topology_node(
        "Mark", "mark", "unknown", "\u2014"
    )
    recorder = render_topology_node(
        "Recorder", "recorder", "unknown", "\u2014"
    )
    maker = render_topology_node(
        "Maker", "maker", "unknown", "\u2014"
    )

    diagram = f"""
<div class="bg-zinc-950 border border-zinc-800 rounded-lg
  p-4 overflow-x-auto"
  hx-get="./x/topology/flow"
  hx-trigger="load, every 2s"
  hx-swap="none"
  hx-on::after-request="applyTopoFlow(event)">

  <div class="text-xs text-zinc-600 mb-3 uppercase
    tracking-wider">RSX System Topology \u2014 click a node</div>

  <div class="flex flex-wrap items-center gap-1 mb-3">
    {client}
    <span class="text-zinc-600 text-xs self-center
      select-none px-1">&#x21C4; WS</span>
    {gateway}
    <span class="text-zinc-600 text-xs self-center
      select-none px-1">&#x2192; CMP</span>
    {risk}
    <span class="text-zinc-600 text-xs self-center
      select-none px-1">&#x2192; CMP</span>
    {matching}
    <span class="text-zinc-600 text-xs self-center
      select-none px-1">&#x2192; WAL</span>
    {marketdata}
  </div>

  <div class="flex flex-wrap items-center gap-3">
    {mark}
    {recorder}
    {maker}
  </div>
</div>

<script>
function applyTopoFlow(evt) {{
  try {{
    var resp = JSON.parse(evt.detail.xhr.responseText);
    resp.nodes.forEach(function(n) {{
      var dot = document.getElementById(
        "topo-dot-" + n.key);
      var rate = document.getElementById(
        "topo-rate-" + n.key);
      if (dot) {{
        dot.className =
          "w-2 h-2 rounded-full " + n.dot + " shrink-0";
      }}
      if (rate) rate.textContent = n.rate;
    }});
  }} catch (e) {{}}
}}
</script>"""

    detail = (
        '<div id="component-detail" '
        'class="text-xs text-zinc-500 italic">'
        "Click a component above to see details."
        "</div>"
    )

    status_bar = (
        '<div class="bg-zinc-900 border border-zinc-800 '
        'rounded px-3 py-2 text-xs font-mono '
        'flex flex-wrap gap-4" '
        'hx-get="./x/topology/summary" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-zinc-600">loading...</span>'
        '</div>'
    )
    content = f"""
<div class="space-y-3">
  {_card("System Topology", diagram + status_bar)}
  {_card("Selected Component", detail)}
</div>"""
    return layout("Topology", content, "./topology")


# ── Screen 3: Book ───────────────────────────────────────

def book_page():
    selector = """
<div class="flex flex-wrap items-center gap-1 mb-3"
  role="group">
  <label class="cursor-pointer">
    <input type="radio" name="book-symbol" value="10"
      class="sr-only peer" checked
      onchange="htmx.trigger('#book-data','load')">
    <span class="peer-checked:bg-blue-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-3 py-1.5 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      PENGU</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="book-symbol" value="3"
      class="sr-only peer"
      onchange="htmx.trigger('#book-data','load')">
    <span class="peer-checked:bg-blue-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-3 py-1.5 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      SOL</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="book-symbol" value="1"
      class="sr-only peer"
      onchange="htmx.trigger('#book-data','load')">
    <span class="peer-checked:bg-blue-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-3 py-1.5 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      BTC</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="book-symbol" value="2"
      class="sr-only peer"
      onchange="htmx.trigger('#book-data','load')">
    <span class="peer-checked:bg-blue-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-3 py-1.5 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      ETH</span>
  </label>
</div>"""
    ladder = _card(
        "Orderbook Ladder",
        f"""{selector}
<div id="book-data" hx-get="./x/book"
     hx-trigger="load, every 1s" hx-swap="innerHTML"
     hx-vals="js:{{symbol_id:
       document.querySelector(
         'input[name=book-symbol]:checked').value}}">
  <span class="text-slate-600">loading...</span>
</div>""",
    )
    stats = _card(
        "Book Stats",
        '<div hx-get="./x/book-stats" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    fills = _card(
        "Live Fills",
        '<div class="max-h-48 overflow-y-auto" '
        'hx-get="./x/live-fills" '
        'hx-trigger="load, every 1s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    agg = _card(
        "Trade Aggregation (1min)",
        '<div hx-get="./x/trade-agg" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    content = f"""
{ladder}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{stats}
{agg}
</div>
{fills}"""
    return layout("Book", content, "./book")


# ── Screen 4: Risk ───────────────────────────────────────

def risk_page():
    overview = _card(
        "Risk Dashboard",
        '<div id="risk-overview-wrap" '
        'hx-get="./x/risk-overview" '
        'hx-trigger="load, every 3s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    latency = _card(
        "Risk Check Latency",
        '<div hx-get="./x/risk-latency" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    lookup = """
<div class="flex flex-wrap items-center gap-2">
  <input type="number" id="risk-uid" name="risk-uid" value="1"
    class="bg-slate-950 border border-slate-700
      text-slate-300 px-2 py-1 rounded text-xs sm:w-20 w-full"
    placeholder="user_id">
  <button class="bg-blue-900/40 text-blue-400
    px-3 py-1 rounded text-xs border border-blue-900
    hover:bg-blue-900 cursor-pointer"
    hx-get="./x/risk-user" hx-include="#risk-uid"
    hx-target="#risk-data"
    hx-swap="innerHTML">Lookup</button>
  <button class="bg-emerald-900/60 text-emerald-400
    px-3 py-1 rounded text-xs border border-emerald-800
    hover:bg-emerald-800 cursor-pointer"
    hx-post="./api/users/create" hx-swap="none">Create User</button>
  <button class="bg-blue-900/40 text-blue-400
    px-3 py-1 rounded text-xs border border-blue-900
    hover:bg-blue-900 cursor-pointer"
    hx-post="./api/users/1/deposit" hx-swap="none"
    >Deposit</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/risk/users/1/freeze"
    hx-swap="none">Freeze</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/risk/users/1/unfreeze"
    hx-swap="none">Unfreeze</button>
  <button class="bg-red-900/40 text-red-400
    px-3 py-1 rounded text-xs border border-red-900
    hover:bg-red-900 cursor-pointer"
    hx-post="./api/risk/liquidate"
    hx-swap="none">Liquidate</button>
</div>
<div id="risk-data" class="mt-3">
  <span class="text-slate-600">
    enter user_id and click lookup</span>
</div>"""
    actions = _card("User Actions", lookup)
    content = f"""
{overview}
{latency}
{actions}"""
    return layout("Risk", content, "./risk")


# ── Screen 5: WAL ────────────────────────────────────────

def wal_page():
    state = _card(
        "Per-Process WAL State",
        '<div hx-get="./x/wal-detail" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    lag = _card(
        "Lag Dashboard (producer seq - consumer seq)",
        '<div hx-get="./x/wal-lag" '
        'hx-trigger="load, every 1s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    rotation = _card(
        "Rotation / Tip Health",
        '<div hx-get="./x/wal-rotation" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    timeline = _card(
        "Timeline (last 100 events)",
        """<div class="flex flex-wrap items-center gap-1 mb-3"
  role="group">
  <label class="cursor-pointer">
    <input type="radio" name="wal-filter-r" value=""
      id="wal-filter" class="sr-only peer" checked>
    <span class="peer-checked:bg-zinc-500
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-2 py-1 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      all</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="wal-filter-r"
      value="ORDER_ACCEPTED" class="sr-only peer">
    <span class="peer-checked:bg-emerald-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-2 py-1 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      ORDER_ACCEPTED</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="wal-filter-r"
      value="FILL" class="sr-only peer">
    <span class="peer-checked:bg-blue-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-2 py-1 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      FILL</span>
  </label>
  <label class="cursor-pointer">
    <input type="radio" name="wal-filter-r"
      value="MARGIN_CHECK" class="sr-only peer">
    <span class="peer-checked:bg-amber-600
      peer-checked:text-white bg-zinc-700 text-zinc-300
      px-2 py-1 rounded text-xs font-medium
      hover:bg-zinc-600 transition-colors block">
      MARGIN_CHECK</span>
  </label>
</div>
<script>
document.querySelectorAll('input[name="wal-filter-r"]')
  .forEach(function(r) {
    r.addEventListener('change', function() {
      document.getElementById('wal-filter').value
        = this.value;
    });
  });
</script>
<div class="max-h-64 overflow-y-auto"
  hx-get="./x/wal-timeline" hx-trigger="load, every 2s"
  hx-swap="innerHTML">
  <span class="text-slate-600">loading...</span>
</div>""",
    )
    files = _card(
        "WAL Files",
        '<div hx-get="./x/wal-files" '
        'hx-trigger="load, every 5s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
        header_right=(
            '<button class="bg-blue-900/40 text-blue-400 '
            'px-3 py-1 rounded text-xs border '
            'border-blue-900 hover:bg-blue-900 '
            'cursor-pointer" '
            'hx-post="./api/wal/verify" '
            'hx-swap="none">Verify</button>'
            '<button class="bg-slate-800 text-slate-400 '
            'px-3 py-1 rounded text-xs border '
            'border-slate-700 hover:bg-slate-700 '
            'cursor-pointer" '
            'hx-post="./api/wal/dump" '
            'hx-swap="none">Dump JSON</button>'
        ),
    )
    content = f"""
{state}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{lag}
{rotation}
</div>
{timeline}
{files}"""
    return layout("WAL", content, "./wal")


# ── Screen 6: Logs ────────────────────────────────────────

def logs_page():
    body = """
<div class="mb-3 space-y-2">
  <div class="flex flex-wrap items-center gap-2">
    <span class="text-xs text-slate-500">Quick filters:</span>
    <button class="bg-slate-800 text-slate-400 px-2 py-1 rounded
      text-xs border border-slate-700 hover:bg-slate-700"
      onclick="quickFilter('gateway', '')">gateway</button>
    <button class="bg-slate-800 text-slate-400 px-2 py-1 rounded
      text-xs border border-slate-700 hover:bg-slate-700"
      onclick="quickFilter('risk', '')">risk</button>
    <button class="bg-slate-800 text-slate-400 px-2 py-1 rounded
      text-xs border border-slate-700 hover:bg-slate-700"
      onclick="quickFilter('matching', '')">matching</button>
    <button class="bg-red-900/40 text-red-400 px-2 py-1 rounded
      text-xs border border-red-900 hover:bg-red-900"
      onclick="quickFilter('', 'error')">errors only</button>
    <button class="bg-amber-900/40 text-amber-400 px-2 py-1 rounded
      text-xs border border-amber-900 hover:bg-amber-900"
      onclick="quickFilter('', 'warn')">warnings only</button>
  </div>
  <div class="flex flex-wrap items-center gap-2">
    <input type="text" id="smart-search"
      class="flex-1 min-w-0 w-full sm:min-w-[200px]
        sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs"
      placeholder="Smart search: 'gateway error order' or just search text (press / to focus, Ctrl+L to clear)"
      onkeydown="handleSmartSearch(event)">
    <button class="bg-slate-800 text-slate-400 px-3 py-1 rounded
      text-xs border border-slate-700 hover:bg-slate-700"
      onclick="clearAllFilters()">Clear All</button>
    <button class="bg-red-900/40 text-red-400 px-3 py-1 rounded
      text-xs border border-red-900 hover:bg-red-900"
      hx-post="./api/logs/clear"
      hx-target="#clear-logs-result"
      hx-swap="innerHTML">Clear All Logs</button>
    <span id="clear-logs-result" class="text-xs"></span>
  </div>
  <div class="hidden" id="filter-dropdowns">
    <select id="log-process" name="log-process"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="">all processes</option>
      <option value="gateway">gateway</option>
      <option value="risk">risk</option>
      <option value="matching">matching</option>
      <option value="marketdata">marketdata</option>
      <option value="mark">mark</option>
      <option value="recorder">recorder</option>
    </select>
    <select id="log-level" name="log-level"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="">all levels</option>
      <option value="error">error</option>
      <option value="warn">warn</option>
      <option value="info">info</option>
      <option value="debug">debug</option>
    </select>
    <input type="text" id="log-search" name="log-search"
      class="w-full sm:w-44 bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs"
      placeholder="search...">
  </div>
</div>
<div id="log-view" class="max-h-[60vh] sm:max-h-[500px]
  overflow-y-auto overflow-x-auto"
     hx-get="./x/logs" hx-trigger="load, every 2s"
     hx-swap="innerHTML"
     hx-include="#log-process, #log-level, #log-search">
  <span class="text-slate-600">loading...</span>
</div>
<div id="log-modal" class="hidden fixed inset-0 bg-black/80
  flex items-center justify-center z-50" onclick="closeModal()">
  <div class="bg-slate-900 border border-slate-700 rounded-lg
    p-4 max-w-[95vw] sm:max-w-4xl max-h-[85vh]
    overflow-auto m-2 sm:m-4"
    onclick="event.stopPropagation()">
    <div class="flex justify-between items-start mb-3">
      <h3 class="text-sm font-semibold text-slate-400">
        Full Log Line</h3>
      <div class="flex gap-2">
        <button class="bg-slate-800 text-slate-400 px-2 py-1
          rounded text-xs hover:bg-slate-700"
          onclick="copyLogLine()">Copy</button>
        <button class="bg-slate-800 text-slate-400 px-2 py-1
          rounded text-xs hover:bg-slate-700"
          onclick="closeModal()">Close</button>
      </div>
    </div>
    <pre id="modal-content"
      class="text-xs text-slate-300 whitespace-pre-wrap
        break-all font-mono bg-slate-950 p-3 rounded"></pre>
  </div>
</div>
<script>
let currentLogLines = [];

function quickFilter(process, level) {
  document.getElementById('log-process').value = process;
  document.getElementById('log-level').value = level;
  document.getElementById('log-search').value = '';
  document.getElementById('smart-search').value = '';
  htmx.trigger('#log-view', 'load');
}

function parseSmartSearch(text) {
  const processes = ['gateway', 'risk', 'matching', 'marketdata',
    'mark', 'recorder'];
  const levels = ['error', 'warn', 'info', 'debug'];
  const words = text.toLowerCase().trim().split(/\s+/);
  let process = '', level = '', search = [];

  for (const word of words) {
    if (processes.includes(word)) {
      process = word;
    } else if (levels.includes(word)) {
      level = word;
    } else {
      search.push(word);
    }
  }

  return { process, level, search: search.join(' ') };
}

function handleSmartSearch(event) {
  if (event.key === 'Enter') {
    const text = event.target.value;
    const parsed = parseSmartSearch(text);
    document.getElementById('log-process').value = parsed.process;
    document.getElementById('log-level').value = parsed.level;
    document.getElementById('log-search').value = parsed.search;
    htmx.trigger('#log-view', 'load');
  }
}

function clearAllFilters() {
  document.getElementById('log-process').value = '';
  document.getElementById('log-level').value = '';
  document.getElementById('log-search').value = '';
  document.getElementById('smart-search').value = '';
  htmx.trigger('#log-view', 'load');
}

function showFullLine(element, index) {
  const modal = document.getElementById('log-modal');
  const content = document.getElementById('modal-content');
  content.textContent = element.textContent.replace('click to expand', '').trim();
  modal.classList.remove('hidden');
}

function closeModal() {
  document.getElementById('log-modal').classList.add('hidden');
}

function copyLogLine() {
  const content = document.getElementById('modal-content').textContent;
  navigator.clipboard.writeText(content).then(() => {
    const btn = event.target;
    const orig = btn.textContent;
    btn.textContent = 'Copied!';
    btn.classList.add('bg-green-900', 'text-green-400');
    setTimeout(() => {
      btn.textContent = orig;
      btn.classList.remove('bg-green-900', 'text-green-400');
    }, 2000);
  });
}

document.addEventListener('keydown', (e) => {
  if (e.key === '/' && e.target.tagName !== 'INPUT') {
    e.preventDefault();
    document.getElementById('smart-search').focus();
  }
  if (e.ctrlKey && e.key === 'l') {
    e.preventDefault();
    clearAllFilters();
  }
  if (e.key === 'Escape') {
    closeModal();
  }
});
</script>"""
    errors = _card(
        "Error Aggregation",
        '<div hx-get="./x/error-agg" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    auth = _card(
        "Auth Failures (last 10)",
        '<div hx-get="./x/auth-failures" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    content = f"""
{_card("Unified Log (last 1000 lines)", body)}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{errors}
{auth}
</div>"""
    return layout("Logs", content, "./logs")


# ── Screen 7: Control ────────────────────────────────────

def control_page():
    scenario_selector = _card(
        "Scenario Selector",
        '''<div class="space-y-3">
  <div class="flex flex-wrap gap-2 items-center">
    <span class="text-xs text-slate-500">Select scenario:</span>
    <select id="scenario-select"
      class="bg-slate-950 border border-slate-700 text-slate-300
        px-2 py-1 rounded text-xs">
      <option value="minimal">minimal (1 symbol, no replication)</option>
      <option value="duo">duo (2 symbols, no replication)</option>
      <option value="full" selected>full (3 symbols, replication)</option>
      <option value="stress-low">stress-low (10 ord/s × 60s)</option>
      <option value="stress-high">stress-high (100 ord/s × 60s)</option>
      <option value="stress-ultra">stress-ultra (500 ord/s × 10s)</option>
    </select>
    <button class="bg-blue-900/40 text-blue-400 px-3 py-1 rounded
      text-xs border border-blue-900 hover:bg-blue-900"
      hx-post="./api/scenario/switch" hx-target="#scenario-status"
      hx-include="#scenario-select">
      Switch Scenario
    </button>
  </div>
  <div id="scenario-status" class="text-xs text-slate-500">
    <span>Current: <code class="bg-slate-800 px-1 rounded"
      hx-get="./x/current-scenario" hx-trigger="load, every 5s"
      hx-swap="innerHTML">loading...</code></span>
  </div>
  <div class="text-[11px] text-slate-600 space-y-1">
    <p><strong class="text-slate-500">Stress scenarios</strong>
      launch load generators after starting processes.</p>
    <p>Monitor latencies in the Orders tab during stress tests.</p>
  </div>
</div>''',
    )
    grid = _card(
        "Process Control",
        '<div hx-get="./x/control-grid" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    notes = _card(
        "Notes",
        '<ul class="text-xs text-slate-500 space-y-1 '
        'list-disc ml-4">'
        '<li>Scenarios: <code class="bg-slate-800 px-1 '
        'rounded">./start full|minimal|stress</code></li>'
        '<li>Clean: <code class="bg-slate-800 px-1 '
        'rounded">./start -c</code></li>'
        '<li>Reset DB: <code class="bg-slate-800 px-1 '
        'rounded">./start --reset-db</code></li></ul>',
    )
    resources = _card(
        "Resource Usage",
        '<div hx-get="./x/resource-usage" '
        'hx-trigger="load, every 5s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    maker = _card(
        "Market Maker",
        '''<div class="space-y-3">
  <div class="flex gap-2">
    <button class="bg-emerald-900/40 text-emerald-400
      px-3 py-1 rounded text-xs border border-emerald-900
      hover:bg-emerald-900"
      hx-post="./api/maker/start" hx-target="#maker-status"
      hx-swap="innerHTML">Start Maker</button>
    <button class="bg-red-900/40 text-red-400
      px-3 py-1 rounded text-xs border border-red-900
      hover:bg-red-900"
      hx-post="./api/maker/stop" hx-target="#maker-status"
      hx-swap="innerHTML">Stop Maker</button>
  </div>
  <div id="maker-status"
    hx-get="./x/maker-status"
    hx-trigger="load, every 3s"
    hx-swap="innerHTML">
    <span class="text-slate-600">loading...</span>
  </div>
  <div class="text-[11px] text-slate-600">
    Places two-sided quotes around mid price via gateway WS.
    Reads BBO from marketdata when available.
  </div>
</div>''',
    )
    return layout(
        "Control",
        scenario_selector + grid + maker + notes + resources,
        "./control")


# ── Screen 8: Faults ─────────────────────────────────────

def faults_page():
    grid = _card(
        "Fault Injection (kill/stop processes)",
        '<div hx-get="./x/faults-grid" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    info = _card(
        "Recovery Notes",
        '<div class="text-xs text-slate-500 space-y-1">'
        '<p>After killing a process, observe recovery via '
        'Overview screen.</p>'
        '<p>For network faults: use '
        '<code class="bg-slate-800 px-1 rounded">'
        'iptables</code> / '
        '<code class="bg-slate-800 px-1 rounded">'
        'tc</code> directly.</p>'
        '<p>WAL corruption: manual hex editing.</p>'
        '</div>',
    )
    return layout("Faults", grid + info, "./faults")


# ── Screen 9: Verify ─────────────────────────────────────

def verify_page():
    invariants = (
        '<div hx-post="./api/verify/run" hx-trigger="load" '
        'hx-target="#verify-results" hx-swap="innerHTML">'
        '</div>'
        '<div id="verify-results" '
        'hx-get="./x/verify" hx-trigger="every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">running checks...</span>'
        '</div>'
    )
    run_btn = (
        '<button class="bg-blue-900/40 text-blue-400 '
        'px-3 py-1 rounded text-xs border border-blue-900 '
        'hover:bg-blue-900 cursor-pointer" '
        'hx-post="./api/verify/run" '
        'hx-target="#verify-results" '
        'hx-swap="innerHTML">Run All Checks</button>'
    )
    recon = _card(
        "Reconciliation",
        '<div hx-get="./x/reconciliation" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    latency = _card(
        "Latency Regression (vs baseline)",
        '<div hx-get="./x/latency-regression" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    note = _card(
        "E2E Tests",
        '<p class="text-xs text-slate-500">'
        'Run E2E tests via '
        '<code class="bg-slate-800 px-1 rounded">'
        'cargo test</code> directly.</p>',
    )
    content = f"""
{_card("Invariants (10 system correctness rules)",
       invariants, header_right=run_btn)}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{recon}
{latency}
</div>
{note}"""
    return layout("Verify", content, "./verify")


# ── Screen 10: Orders ────────────────────────────────────

def orders_page():
    # ── quick-order matrix ─────────────────────────────
    _buy_cls = (
        "bg-emerald-600 hover:bg-emerald-500 "
        "text-white border-emerald-700"
    )
    _sell_cls = (
        "bg-red-600 hover:bg-red-500 "
        "text-white border-red-700"
    )
    _rand_cls = (
        "bg-violet-600 hover:bg-violet-500 "
        "text-white border-violet-700"
    )
    _btn_base = (
        "text-sm font-bold py-3 px-2 rounded border "
        "cursor-pointer w-full min-h-[48px]"
    )

    def _qbtn(label, vals_json, color_cls):
        return (
            f'<button class="{color_cls} {_btn_base}" '
            f'hx-post="./api/orders/quick" '
            f'hx-target="#quick-result" '
            f'hx-swap="innerHTML" '
            f"hx-vals='{vals_json}'>"
            f"{label}</button>"
        )

    def _mkt(label, side, qty):
        cls = _buy_cls if side == "buy" else _sell_cls
        return _qbtn(
            label,
            (
                f'{{"side":"{side}","qty":"{qty}",'
                f'"price_offset_pct":"0"}}'
            ),
            cls,
        )

    def _lmt(label, side, qty, pct):
        cls = _buy_cls if side == "buy" else _sell_cls
        return _qbtn(
            label,
            (
                f'{{"side":"{side}","qty":"{qty}",'
                f'"price_offset_pct":"{pct}"}}'
            ),
            cls,
        )

    multipliers = ["1", "5", "20", "100"]

    buy_row = "".join(
        _mkt(f"{q}x", "buy", q) for q in multipliers
    )
    sell_row = "".join(
        _mkt(f"{q}x", "sell", q) for q in multipliers
    )
    rand_row = (
        _qbtn(
            "\U0001f3b2 Random",
            '{"randomize":"true"}',
            _rand_cls,
        )
        + _qbtn(
            "\U0001f3b2 Rand Side",
            '{"rand_side":"true","qty":"5"}',
            _rand_cls,
        )
    )

    matrix_html = (
        '<div class="space-y-2 mb-2">'
        '<div class="text-[10px] text-slate-500 uppercase'
        ' tracking-wider">Buy (up)</div>'
        f'<div class="grid grid-cols-4 gap-2">'
        f'{buy_row}</div>'
        '<div class="text-[10px] text-slate-500 uppercase'
        ' tracking-wider mt-1">Sell (down)</div>'
        f'<div class="grid grid-cols-4 gap-2">'
        f'{sell_row}</div>'
        f'<div class="grid grid-cols-2 gap-2 mt-1">'
        f'{rand_row}</div>'
        '</div>'
        '<div id="quick-result" '
        'class="text-xs min-h-[20px] mb-3"></div>'
    )

    custom_form = """<details class="group">
  <summary class="cursor-pointer text-xs text-slate-400
    hover:text-slate-200 select-none py-1 mb-2
    list-none flex items-center gap-1">
    <span class="group-open:hidden">&#9654; Custom Order</span>
    <span class="hidden group-open:inline"
      >&#9660; Custom Order</span>
  </summary>
<form hx-post="./api/orders/test"
      hx-target="#order-result"
      hx-swap="innerHTML"
      class="flex flex-wrap gap-3 items-end mb-3">
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Symbol</label>
    <select name="symbol_id"
      class="w-full sm:w-auto bg-slate-950 border
        border-slate-700 text-slate-300 px-2 py-1
        rounded text-xs">
      <option value="10">PENGU</option>
      <option value="3">SOL</option>
      <option value="1">BTC</option>
      <option value="2">ETH</option>
    </select>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Side</label>
    <div class="flex gap-1 flex-wrap" role="group">
      <label class="cursor-pointer">
        <input type="radio" name="side" value="buy"
          class="sr-only peer" checked>
        <span class="peer-checked:bg-emerald-600
          peer-checked:text-white bg-zinc-700
          text-zinc-300 px-3 py-1.5 rounded text-xs
          font-medium hover:bg-zinc-600
          transition-colors block">Buy</span>
      </label>
      <label class="cursor-pointer">
        <input type="radio" name="side" value="sell"
          class="sr-only peer">
        <span class="peer-checked:bg-red-600
          peer-checked:text-white bg-zinc-700
          text-zinc-300 px-3 py-1.5 rounded text-xs
          font-medium hover:bg-zinc-600
          transition-colors block">Sell</span>
      </label>
    </div>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Price</label>
    <input type="text" name="price" value="0"
      placeholder="0 = market"
      class="w-full sm:w-28 bg-slate-950 border
        border-slate-700 text-slate-300 px-2 py-1
        rounded text-xs">
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Qty</label>
    <input type="text" name="qty" value="1.0"
      class="w-full sm:w-20 bg-slate-950 border
        border-slate-700 text-slate-300 px-2 py-1
        rounded text-xs">
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">TIF</label>
    <div class="flex gap-1 flex-wrap" role="group">
      <label class="cursor-pointer">
        <input type="radio" name="tif" value="GTC"
          class="sr-only peer" checked>
        <span class="peer-checked:bg-blue-600
          peer-checked:text-white bg-zinc-700
          text-zinc-300 px-3 py-1.5 rounded text-xs
          font-medium hover:bg-zinc-600
          transition-colors block">GTC</span>
      </label>
      <label class="cursor-pointer">
        <input type="radio" name="tif" value="IOC"
          class="sr-only peer">
        <span class="peer-checked:bg-amber-600
          peer-checked:text-white bg-zinc-700
          text-zinc-300 px-3 py-1.5 rounded text-xs
          font-medium hover:bg-zinc-600
          transition-colors block">IOC</span>
      </label>
    </div>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">User ID</label>
    <input type="number" name="user_id" value="1"
      class="w-full sm:w-20 bg-slate-950 border
        border-slate-700 text-slate-300 px-2 py-1
        rounded text-xs">
  </div>
  <div class="flex items-center gap-3">
    <label class="cursor-pointer flex items-center gap-2
      min-h-[44px]">
      <input type="checkbox" name="reduce_only"
        class="sr-only peer">
      <div class="w-8 h-4 bg-zinc-600 rounded-full
        peer-checked:bg-emerald-500 relative
        after:absolute after:top-0.5 after:left-0.5
        after:w-3 after:h-3 after:bg-white
        after:rounded-full after:transition-all
        peer-checked:after:translate-x-4"></div>
      <span class="text-xs text-zinc-300">Reduce Only</span>
    </label>
  </div>
  <button type="submit"
    class="w-full sm:w-auto bg-emerald-900/60
      text-emerald-400 px-4 py-2 rounded text-xs
      border border-emerald-800 hover:bg-emerald-800
      cursor-pointer min-h-[44px]">Submit</button>
</form>
<div class="flex flex-wrap gap-2 mb-3">
  <button class="bg-slate-800 text-slate-400
    px-3 py-1.5 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/batch"
    hx-target="#order-result"
    hx-swap="innerHTML">Batch (10)</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1.5 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/random"
    hx-target="#order-result"
    hx-swap="innerHTML">Random (5)</button>
  <button class="bg-amber-900/40 text-amber-400
    px-3 py-1.5 rounded text-xs border border-amber-900
    hover:bg-amber-900 cursor-pointer"
    hx-post="./api/stress/run"
    hx-target="#order-result"
    hx-swap="innerHTML">Stress (100)</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1.5 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/invalid"
    hx-target="#order-result"
    hx-swap="innerHTML">Invalid</button>
</div>
<div id="order-result"></div>
</details>"""

    form = matrix_html + custom_form

    lifecycle = _card(
        "Order Lifecycle Trace",
        """<div class="flex flex-wrap items-center gap-2 mb-3">
  <input type="text" id="trace-oid" name="trace-oid"
    placeholder="oid..."
    class="w-full sm:w-48 bg-slate-950 border border-slate-700
      text-slate-300 px-2 py-1 rounded text-xs">
  <button class="w-full sm:w-auto bg-blue-900/40 text-blue-400
    px-3 py-1 rounded text-xs border border-blue-900
    hover:bg-blue-900 cursor-pointer"
    hx-get="./x/order-trace"
    hx-include="#trace-oid"
    hx-target="#trace-result"
    hx-swap="innerHTML">Trace</button>
</div>
<div id="trace-result">
  <span class="text-slate-600">
    enter oid and click trace</span>
</div>""",
    )
    orders_list = _card(
        "Recent Orders (last 50)",
        '<div hx-get="./x/recent-orders" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    stale = _card(
        "Stale Orders (>1 hour unfilled)",
        '<div hx-get="./x/stale-orders" '
        'hx-trigger="load, every 10s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    content = f"""
{_card("Submit Order", form)}
{lifecycle}
{orders_list}
{stale}"""
    return layout("Orders", content, "./orders")


# ── HTMX partial renderers ─────────────────────────────


def _percentile(data, p):
    """Linear interpolation percentile on sorted data."""
    k = (len(data) - 1) * p / 100
    f = int(k)
    c = f + 1
    if c >= len(data):
        return data[-1]
    return data[f] + (k - f) * (data[c] - data[f])


_TH = ('class="text-left py-1.5 px-2 text-[10px] '
       'text-slate-500 uppercase tracking-wider '
       'border-b border-slate-800 font-semibold"')
_TD = ('class="py-1.5 px-2 text-xs border-b '
       'border-slate-800/50"')


def _table(headers, rows_html):
    ths = "".join(f"<th {_TH}>{h}</th>" for h in headers)
    return (
        f'<div class="overflow-x-auto">'
        f'<table class="w-full table-auto">'
        f'<thead><tr>{ths}</tr></thead>'
        f'<tbody>{rows_html}</tbody></table>'
        f'</div>'
    )


def _dot(state):
    if state == "running":
        return ('<span class="inline-block w-2 h-2 '
                'rounded-full bg-emerald-500 '
                'shadow-[0_0_4px_#22c55e66]"></span>')
    return ('<span class="inline-block w-2 h-2 '
            'rounded-full bg-red-500 '
            'shadow-[0_0_4px_#ef444466]"></span>')


def _btn(label, cls, extra=""):
    """Small action button."""
    colors = {
        "green": ("bg-emerald-900/60 text-emerald-400 "
                  "border-emerald-800 hover:bg-emerald-800"),
        "red": ("bg-red-900/40 text-red-400 "
                "border-red-900 hover:bg-red-900"),
        "blue": ("bg-blue-900/40 text-blue-400 "
                 "border-blue-900 hover:bg-blue-900"),
    }
    c = colors.get(cls, colors["blue"])
    return (f'<button class="{c} px-2 py-0.5 rounded '
            f'text-[10px] border cursor-pointer" '
            f'{extra}>{label}</button>')


_BAR_BG = {
    "emerald": "bg-emerald-500",
    "amber": "bg-amber-500",
    "red": "bg-red-500",
}


def _bar(pct, color="emerald"):
    """Progress bar for backpressure / resource usage."""
    if pct > 80:
        color = "red"
    elif pct > 50:
        color = "amber"
    w = max(1, min(100, pct))
    bg = _BAR_BG.get(color, "bg-emerald-500")
    return (
        f'<div class="flex items-center gap-2">'
        f'<div class="flex-1 bg-slate-800 rounded h-2">'
        f'<div class="{bg} h-2 rounded" '
        f'style="width:{w}%"></div></div>'
        f'<span class="text-[10px] text-slate-500 w-8 '
        f'text-right">{pct}%</span></div>'
    )


_METRIC_TEXT = {
    "slate-300": "text-slate-300",
    "slate-500": "text-slate-500",
    "blue-400": "text-blue-400",
    "emerald-400": "text-emerald-400",
    "amber-400": "text-amber-400",
    "red-400": "text-red-400",
}


def _metric(label, value, color="slate-300"):
    """Single metric in a strip."""
    cls = _METRIC_TEXT.get(color, "text-slate-300")
    return (
        f'<div class="text-center">'
        f'<div class="text-[10px] text-slate-500 '
        f'uppercase tracking-wider">{label}</div>'
        f'<div class="text-sm {cls} '
        f'font-semibold">{value}</div></div>'
    )


# ── Health (overview) ────────────────────────────────────

def render_health(procs, pg_ok):
    running = sum(
        1 for p in procs if p.get("state") == "running")
    total = max(len(procs), 1)
    score = int((running / total) * 100) if procs else 0
    if pg_ok:
        score = min(100, score + 5)
    _HEALTH_BG = {
        "emerald": "bg-emerald-500",
        "amber": "bg-amber-500",
        "red": "bg-red-500",
    }
    _HEALTH_TEXT = {
        "emerald": "text-emerald-400",
        "amber": "text-amber-400",
        "red": "text-red-400",
    }
    if score >= 80:
        color = "emerald"
    elif score >= 50:
        color = "amber"
    else:
        color = "red"
    bar_w = max(1, score)
    bg = _HEALTH_BG[color]
    text = _HEALTH_TEXT[color]
    label = "green" if score >= 80 else "yellow" if score >= 50 else "red"
    return (
        f'<div class="flex items-center gap-4">'
        f'<div class="flex-1 bg-slate-800 rounded h-4">'
        f'<div class="{bg} h-4 rounded flex '
        f'items-center justify-center text-[10px] '
        f'font-bold text-white" '
        f'style="width:{bar_w}%">{score}</div></div>'
        f'<span class="text-xs font-semibold '
        f'{text} uppercase">'
        f'{label}'
        f'</span></div>'
    )


def render_key_metrics(
    procs, wal_streams,
    active_orders=0, positions=0, msgs_sec=0,
):
    running = sum(
        1 for p in procs if p.get("state") == "running")
    wal_files = sum(s.get("files", 0) for s in wal_streams)
    ao_color = "blue-400" if active_orders else "slate-500"
    pos_color = "amber-400" if positions else "slate-500"
    ms_color = "cyan-400" if msgs_sec else "slate-500"
    return (
        '<div class="grid grid-cols-2 sm:grid-cols-3 '
        'md:grid-cols-6 gap-4">'
        + _metric("Processes", f"{running}/{len(procs)}",
                  "emerald-400")
        + _metric("Active Orders",
                  str(active_orders), ao_color)
        + _metric("Positions",
                  str(positions), pos_color)
        + _metric("Msgs/sec",
                  str(msgs_sec), ms_color)
        + _metric("WAL Files", str(wal_files), "blue-400")
        + _metric("Errors", "0", "emerald-400")
        + '</div>'
    )


def render_ring_pressure(streams=None):
    """Derive ring fill % from WAL stream lag."""
    ring_map = {
        "GW -> Risk": "gateway",
        "Risk -> ME": "risk",
        "ME -> Mktdata": "me",
        "ME -> Recorder": "recorder",
    }
    ring_pcts = {k: 0 for k in ring_map}
    if streams:
        for s in streams:
            sname = s.get("name", "").lower()
            for label, pattern in ring_map.items():
                if pattern in sname:
                    lag = s.get("lag_mb")
                    if lag is not None:
                        pct = min(100, int(lag * 10))
                    else:
                        pct = min(100,
                                  s.get("files", 0) * 5)
                    ring_pcts[label] = max(
                        ring_pcts[label], pct)
    rings = [(k, ring_pcts[k]) for k in ring_map]
    html = '<div class="grid grid-cols-1 md:grid-cols-2 gap-3">'
    for name, pct in rings:
        html += (
            f'<div>'
            f'<div class="text-[10px] text-slate-500 mb-1">'
            f'{name}</div>'
            + _bar(pct) +
            '</div>'
        )
    html += '</div>'
    return html


def render_invariant_status(checks):
    if not checks:
        return ('<span class="text-emerald-400 text-xs">'
                'All passing</span>'
                '<span class="text-slate-600 text-xs ml-2">'
                '(run checks on Verify tab)</span>')
    fails = [c for c in checks if c["status"] == "fail"]
    if fails:
        return (
            f'<span class="text-red-400 text-xs">'
            f'{len(fails)} violation(s)</span>')
    return ('<span class="text-emerald-400 text-xs">'
            'All passing</span>')


# ── Process table ────────────────────────────────────────

def render_process_table(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found — click '
                '"Build &amp; Start All" above</span>')
    rows = ""
    cards = ""
    for p in processes:
        name = p["name"]
        state = p.get("state", "unknown")
        if state == "running":
            actions = (
                _btn("Restart", "blue",
                     f'hx-post="./api/processes/{name}/'
                     f'restart" hx-swap="none"') + " "
                + _btn("Stop", "red",
                       f'hx-post="./api/processes/{name}/'
                       f'stop" hx-swap="none"')
            )
        else:
            actions = _btn(
                "Start", "green",
                f'hx-post="./api/processes/{name}/start" '
                'hx-swap="none"',
            )
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{_dot(state)}</td>'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{p.get("pid", "-")}</td>'
            f'<td {_TD}>{p.get("cpu", "-")}</td>'
            f'<td {_TD}>{p.get("mem", "-")}</td>'
            f'<td {_TD}>{p.get("uptime", "-")}</td>'
            f'<td {_TD}>{state}</td>'
            f'<td {_TD}>{actions}</td></tr>'
        )
        cards += (
            f'<div class="border border-slate-800 rounded'
            f' px-3 py-2 flex items-center'
            f' justify-between gap-2">'
            f'<div class="flex items-center gap-2 min-w-0">'
            f'{_dot(state)}'
            f'<span class="text-xs font-mono">{name}</span>'
            f'<span class="text-[10px] text-slate-500'
            f'">{state}</span>'
            f'</div>'
            f'<div class="flex gap-1 flex-shrink-0">'
            f'{actions}</div>'
            f'</div>'
        )
    table_html = _table(
        ["", "Name", "PID", "CPU%", "Mem",
         "Uptime", "State", "Actions"],
        rows,
    )
    return (
        f'<div class="hidden sm:block">{table_html}</div>'
        f'<div class="sm:hidden space-y-1">{cards}</div>'
    )


# ── Core affinity (topology) ────────────────────────────

def render_core_affinity(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes</span>')
    html = '<div class="flex flex-wrap gap-2">'
    for i, p in enumerate(processes):
        state = p.get("state", "unknown")
        bg = ("bg-emerald-900/30 border-emerald-800"
              if state == "running"
              else "bg-slate-800 border-slate-700")
        html += (
            f'<div class="{bg} border rounded px-3 py-2 '
            f'text-center">'
            f'<div class="text-[10px] text-slate-500">'
            f'Core {i}</div>'
            f'<div class="text-xs">{p["name"]}</div>'
            f'</div>'
        )
    html += '</div>'
    return html


def render_cmp_flows(record_counts=None):
    """CMP flow stats from WAL record counts."""
    if record_counts:
        f = record_counts.get("fills", 0)
        b = record_counts.get("bbos", 0)
        flows = [
            ("Gateway -> Risk",
             str(f + b), str(f + b), "0", "0"),
            ("Risk -> ME",
             str(f), str(f), "0", "0"),
            ("ME -> Mktdata",
             str(b), str(b), "0", "0"),
        ]
    else:
        flows = [
            ("Gateway -> Risk", "--", "--", "0", "0"),
            ("Risk -> ME", "--", "--", "0", "0"),
            ("ME -> Mktdata", "--", "--", "0", "0"),
        ]
    rows = ""
    for name, sent, recv, nak, drop in flows:
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{sent}</td>'
            f'<td {_TD}>{recv}</td>'
            f'<td {_TD}>{nak}</td>'
            f'<td {_TD}>{drop}</td></tr>'
        )
    return _table(
        ["Connection", "Sent", "Recv", "NAK", "Drop"],
        rows,
    )


# ── Control grid ─────────────────────────────────────────

def render_control_grid(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found</span>')
    rows = ""
    cards = ""
    for p in processes:
        name = p["name"]
        state = p.get("state", "unknown")
        if state == "running":
            actions = (
                _btn("Stop", "red",
                     f'hx-post="./api/processes/{name}/stop" '
                     'hx-swap="none"') + " "
                + _btn("Restart", "blue",
                       f'hx-post="./api/processes/{name}/'
                       f'restart" hx-swap="none"') + " "
                + _btn("Kill", "red",
                       f'hx-post="./api/processes/{name}/'
                       f'kill" hx-swap="none"')
            )
        else:
            actions = _btn(
                "Start", "green",
                f'hx-post="./api/processes/{name}/start" '
                'hx-swap="none"',
            )
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{_dot(state)}</td>'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{state}</td>'
            f'<td {_TD}>{p.get("pid", "-")}</td>'
            f'<td {_TD}>{p.get("uptime", "-")}</td>'
            f'<td {_TD}>{actions}</td></tr>'
        )
        cards += (
            f'<div class="border border-slate-800 rounded'
            f' px-3 py-2 flex items-center'
            f' justify-between gap-2">'
            f'<div class="flex items-center gap-2 min-w-0">'
            f'{_dot(state)}'
            f'<span class="text-xs font-mono">{name}</span>'
            f'<span class="text-[10px] text-slate-500">'
            f'{p.get("pid", "-")}</span>'
            f'</div>'
            f'<div class="flex flex-wrap gap-1 flex-shrink-0">'
            f'{actions}</div>'
            f'</div>'
        )
    table_html = _table(
        ["", "Name", "State", "PID", "Uptime", "Actions"],
        rows,
    )
    return (
        f'<div class="hidden sm:block">{table_html}</div>'
        f'<div class="sm:hidden space-y-1">{cards}</div>'
    )


def render_resource_usage(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found</span>')
    rows = ""
    for p in processes:
        if p.get("state") != "running":
            continue
        cpu_str = p.get("cpu", "0%").rstrip("%")
        try:
            cpu = float(cpu_str)
        except ValueError:
            cpu = 0
        mem = p.get("mem", "-")
        rows += (
            f'<div>'
            f'<div class="flex items-center gap-3 mb-1">'
            f'<span class="text-xs w-24">{p["name"]}</span>'
            f'<span class="text-[10px] text-slate-500 '
            f'w-16">CPU {p.get("cpu", "-")}</span>'
            f'<div class="flex-1">{_bar(int(cpu))}</div>'
            f'<span class="text-[10px] text-slate-500 '
            f'w-16">Mem {mem}</span>'
            f'</div></div>'
        )
    if not rows:
        return ('<span class="text-slate-600">'
                'no running processes</span>')
    return f'<div class="space-y-3">{rows}</div>'


# ── Faults grid ──────────────────────────────────────────

def render_faults_grid(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found</span>')
    rows = ""
    cards = ""
    for p in processes:
        name = p["name"]
        state = p.get("state", "unknown")
        actions = (
            _btn("Stop", "red",
                 f'hx-post="./api/processes/{name}/stop" '
                 'hx-swap="none"') + " "
            + _btn("Kill", "red",
                   f'hx-post="./api/processes/{name}/kill" '
                   'hx-swap="none"')
        )
        if state != "running":
            actions += " " + _btn(
                "Restart", "blue",
                f'hx-post="./api/processes/{name}/restart" '
                'hx-swap="none"',
            )
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{_dot(state)}</td>'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{state}</td>'
            f'<td {_TD}>{p.get("pid", "-")}</td>'
            f'<td {_TD}>{actions}</td></tr>'
        )
        cards += (
            f'<div class="border border-slate-800 rounded'
            f' px-3 py-2 flex items-center'
            f' justify-between gap-2">'
            f'<div class="flex items-center gap-2 min-w-0">'
            f'{_dot(state)}'
            f'<span class="text-xs font-mono">{name}</span>'
            f'<span class="text-[10px] text-slate-500">'
            f'{state}</span>'
            f'</div>'
            f'<div class="flex flex-wrap gap-1 flex-shrink-0">'
            f'{actions}</div>'
            f'</div>'
        )
    table_html = _table(
        ["", "Name", "State", "PID", "Actions"], rows,
    )
    return (
        f'<div class="hidden sm:block">{table_html}</div>'
        f'<div class="sm:hidden space-y-1">{cards}</div>'
    )


# ── WAL renderers ────────────────────────────────────────

def render_wal_status(streams):
    if not streams:
        return ('<span class="text-slate-600">'
                'no WAL streams found</span>')
    rows = ""
    for s in streams:
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{s.get("name", "-")}</td>'
            f'<td {_TD}>{s.get("files", "-")}</td>'
            f'<td {_TD}>{s.get("total_size", "-")}</td></tr>'
        )
    return _table(["Stream", "Files", "Size"], rows)


def render_wal_detail(streams):
    if not streams:
        return ('<span class="text-slate-600">'
                'no WAL streams found</span>')
    rows = ""
    for s in streams:
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{s.get("name", "-")}</td>'
            f'<td {_TD}>{s.get("files", "-")}</td>'
            f'<td {_TD}>{s.get("total_size", "-")}</td>'
            f'<td {_TD}>{s.get("newest", "-")}</td></tr>'
        )
    return _table(
        ["Stream", "Files", "Size", "Newest"], rows,
    )


def render_wal_files(files):
    if not files:
        return ('<span class="text-slate-600">'
                'no WAL files found</span>')
    rows = ""
    for f in files:
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{f.get("stream", "-")}</td>'
            f'<td {_TD}>{f.get("name", "-")}</td>'
            f'<td {_TD}>{f.get("size", "-")}</td>'
            f'<td {_TD}>{f.get("modified", "-")}</td></tr>'
        )
    return _table(
        ["Stream", "File", "Size", "Modified"], rows,
    )


def render_wal_lag(streams=None):
    """Show WAL producer-consumer lag per stream."""
    if not streams:
        return ('<span class="text-slate-500 text-xs">'
                'no WAL streams found</span>')
    rows = ""
    for s in streams:
        name = s.get("name", "-")
        size = s.get("total_size", "0 B")
        newest = s.get("newest", "--")
        files = s.get("files", 0)
        # lag indicator based on file count
        if files == 0:
            lag = '<span class="text-slate-500">idle</span>'
        elif newest == "--":
            lag = '<span class="text-amber-400">stale</span>'
        else:
            lag = ('<span class="text-emerald-400">'
                   'active</span>')
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{size}</td>'
            f'<td {_TD}>{newest}</td>'
            f'<td {_TD}>{lag}</td></tr>'
        )
    return _table(
        ["Stream", "Size", "Last Write", "Status"], rows,
    )


def render_wal_rotation(streams=None):
    """Show WAL rotation and tip health per stream."""
    if not streams:
        return ('<span class="text-slate-500 text-xs">'
                'no WAL streams found</span>')
    rows = ""
    for s in streams:
        name = s.get("name", "-")
        files = s.get("files", 0)
        size = s.get("total_size", "0 B")
        # rotation status based on file count
        if files == 0:
            status = ('<span class="text-slate-500">'
                      'empty</span>')
        elif files == 1:
            status = ('<span class="text-emerald-400">'
                      '1 active</span>')
        else:
            status = (f'<span class="text-amber-400">'
                      f'{files} files (pending rotation)'
                      f'</span>')
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{name}</td>'
            f'<td {_TD}>{files}</td>'
            f'<td {_TD}>{size}</td>'
            f'<td {_TD}>{status}</td></tr>'
        )
    return _table(
        ["Stream", "Files", "Size", "Rotation"], rows,
    )


def render_wal_timeline(records=None):
    """Show recent WAL events as timeline."""
    if not records:
        return ('<span class="text-slate-500 text-xs">'
                'no WAL events recorded</span>')
    rows = ""
    for rec in records[:50]:
        rtype = rec.get("type", "?")
        seq = rec.get("seq", 0)
        sid = rec.get("symbol_id", 0)
        sym = {1: "BTC", 2: "ETH", 3: "SOL",
               10: "PENGU"}.get(sid, f"sym-{sid}")
        if rtype == "bbo":
            color = "text-blue-400"
            detail = (f'bid={rec.get("bid_px", 0)} '
                      f'ask={rec.get("ask_px", 0)}')
        elif rtype == "fill":
            color = "text-emerald-400"
            detail = (f'px={rec.get("price", 0)} '
                      f'qty={rec.get("qty", 0)}')
        else:
            color = "text-slate-400"
            detail = "-"
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{seq}</td>'
            f'<td {_TD}><span class="{color}">'
            f'{rtype.upper()}</span></td>'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD} class="text-slate-400 text-xs">'
            f'{html.escape(detail)}</td></tr>'
        )
    return _table(["Seq", "Type", "Symbol", "Detail"], rows)


# ── Logs ─────────────────────────────────────────────────

def render_logs(lines):
    if not lines:
        return ('<span class="text-slate-600">'
                'no log lines</span>')
    out = ""
    for i, line in enumerate(lines):
        cls = "text-slate-400"
        low = line.lower()
        if " error " in low:
            cls = "text-red-400"
        elif " warn " in low:
            cls = "text-amber-400"
        elif " debug " in low:
            cls = "text-slate-600"
        safe_line = html.escape(line)
        out += (
            f'<div class="{cls} text-xs py-0.5 font-mono '
            f'whitespace-pre-wrap break-all cursor-pointer '
            f'hover:bg-slate-800 px-1 rounded group relative" '
            f'onclick="showFullLine(this, {i})">'
            f'<span class="group-hover:opacity-100 opacity-0 '
            f'absolute right-1 top-1 text-[10px] text-slate-500">'
            f'click to expand</span>'
            f'{safe_line}</div>\n'
        )
    return out


def render_error_agg(lines):
    """Group error lines by pattern, show count."""
    errors = {}
    for line in lines:
        if " error " in line.lower():
            # crude pattern: first 50 chars
            key = line[:60].strip()
            if key not in errors:
                errors[key] = {"count": 0,
                               "first": line, "last": line}
            errors[key]["count"] += 1
            errors[key]["last"] = line
    if not errors:
        return ('<span class="text-slate-600 text-xs">'
                'no errors</span>')
    rows = ""
    for pattern, info in list(errors.items())[:20]:
        truncated = pattern[:60]
        safe = html.escape(truncated)
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD} class="text-red-400 text-xs '
            f'max-w-xs truncate">{safe}</td>'
            f'<td {_TD}>{info["count"]}</td></tr>'
        )
    return _table(["Pattern", "Count"], rows)


# ── Verify ───────────────────────────────────────────────

def render_verify(checks):
    if not checks:
        return ('<span class="text-slate-600">'
                'no checks run yet</span>')
    rows = ""
    for c in checks:
        if c.get("status") == "pass":
            badge = ("bg-emerald-950 text-emerald-400 "
                     "border border-emerald-900")
            label = "PASS"
        elif c.get("status") == "fail":
            badge = ("bg-red-950 text-red-400 "
                     "border border-red-900")
            label = "FAIL"
        elif c.get("status") == "warn":
            badge = ("bg-yellow-950 text-yellow-400 "
                     "border border-yellow-900")
            label = "WARN"
        else:
            badge = ("bg-slate-800 text-slate-400 "
                     "border border-slate-700")
            label = "SKIP"
        detail = ""
        if c.get("detail"):
            detail = (
                f'<div class="text-[10px] text-slate-500 '
                f'mt-0.5">{html.escape(str(c["detail"]))}</div>'
            )
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}><span class="{badge} px-2 py-0.5 '
            f'rounded text-[10px] font-semibold">'
            f'{label}</span></td>'
            f'<td {_TD}>{html.escape(str(c.get("name", "-")))}{detail}</td>'
            f'<td {_TD} class="text-slate-500 text-[10px]">'
            f'{html.escape(str(c.get("time", "-")))}</td></tr>'
        )
    return _table(["Status", "Check", "Last Run"], rows)


def render_reconciliation(
    shadow_vs_me=None, mark_vs_index=None,
):
    """Reconciliation checks with optional live data."""
    shadow_item = ("Shadow book vs ME book",
                   shadow_vs_me[0], shadow_vs_me[1]) \
        if shadow_vs_me else \
        ("Shadow book vs ME book", "skip",
         "requires live system")
    mark_item = ("Mark price vs index",
                 mark_vs_index[0], mark_vs_index[1]) \
        if mark_vs_index else \
        ("Mark price vs index", "skip",
         "requires live system")
    items = [
        ("Frozen margin vs computed", "skip",
         "requires live system"),
        shadow_item,
        mark_item,
    ]
    rows = ""
    for name, status, detail in items:
        if status == "pass":
            badge = ("bg-emerald-950 text-emerald-400 "
                     "border border-emerald-900")
        elif status == "fail":
            badge = ("bg-red-950 text-red-400 "
                     "border border-red-900")
        else:
            badge = ("bg-amber-950 text-amber-400 "
                     "border border-amber-900")
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}><span class="{badge} px-2 py-0.5 '
            f'rounded text-[10px] font-semibold">'
            f'{status.upper()}</span></td>'
            f'<td {_TD}>{name}'
            f'<div class="text-[10px] text-slate-500 '
            f'mt-0.5">{detail}</div></td></tr>'
        )
    return _table(["Status", "Check"], rows)


def render_latency_regression(latencies=None):
    """Show latency regression vs baseline targets."""
    baseline_gw_p99 = 50  # us
    baseline_me_p99 = 0.5  # us (500ns)

    if not latencies or len(latencies) == 0:
        return (
            '<div class="space-y-2">'
            '<div class="flex items-center gap-3">'
            '<span class="text-xs w-40">GW->ME->GW p99</span>'
            '<span class="text-slate-500 text-xs">--</span>'
            '<span class="text-[10px] text-slate-600">'
            f'(baseline {baseline_gw_p99}us)</span></div>'
            '<div class="flex items-center gap-3">'
            '<span class="text-xs w-40">ME match p99</span>'
            '<span class="text-slate-500 text-xs">--</span>'
            '<span class="text-[10px] text-slate-600">'
            f'(baseline {int(baseline_me_p99*1000)}ns)</span></div>'
            '</div>'
        )

    sorted_lat = sorted(latencies)
    p99 = int(_percentile(sorted_lat, 99))
    delta = p99 - baseline_gw_p99
    pct_change = (delta / baseline_gw_p99) * 100 if baseline_gw_p99 > 0 else 0

    if delta < 0:
        delta_str = f'<span class="text-emerald-400">{delta}us ({pct_change:.0f}%)</span>'
    elif delta < baseline_gw_p99 * 0.1:
        delta_str = f'<span class="text-amber-400">+{delta}us (+{pct_change:.0f}%)</span>'
    else:
        delta_str = f'<span class="text-red-400">+{delta}us (+{pct_change:.0f}%)</span>'

    return (
        '<div class="space-y-2">'
        '<div class="flex items-center gap-3">'
        '<span class="text-xs w-40">GW->ME->GW p99</span>'
        f'<span class="text-slate-300 text-xs">{p99}us</span>'
        f'{delta_str}'
        '<span class="text-[10px] text-slate-600">'
        f'(baseline {baseline_gw_p99}us)</span></div>'
        '<div class="flex items-center gap-3">'
        '<span class="text-xs w-40">ME match p99</span>'
        '<span class="text-slate-500 text-xs">--</span>'
        '<span class="text-[10px] text-slate-600">'
        f'(baseline {int(baseline_me_p99*1000)}ns)</span></div>'
        '</div>'
    )


# ── Orders ───────────────────────────────────────────────

SYMBOL_NAMES_STR = {
    "1": "BTC", "2": "ETH", "3": "SOL", "10": "PENGU",
}


def render_recent_orders(orders):
    if not orders:
        return ('<span class="text-slate-600">'
                'no orders yet</span>')
    rows = ""
    for o in orders:
        sym = o.get("symbol", "-")
        sym_name = SYMBOL_NAMES_STR.get(sym, sym)
        tif = o.get("tif", "GTC")
        flags = ""
        if o.get("reduce_only"):
            flags += "RO "
        if o.get("post_only"):
            flags += "PO"
        cancel = ""
        if o.get("status") == "submitted":
            cid = o.get("cid", "")
            cancel = (
                f'<button class="bg-red-900/40 text-red-400 '
                f'px-2 py-0.5 rounded text-[10px] border '
                f'border-red-900 hover:bg-red-900 '
                f'cursor-pointer" '
                f'hx-post="./api/orders/{cid}/cancel" '
                f'hx-target="#order-result" '
                f'hx-swap="innerHTML">Cancel</button>'
            )

        latency_str = ""
        if o.get("latency_us"):
            lat = o["latency_us"]
            if lat < 100:
                color = "text-emerald-400"
            elif lat < 500:
                color = "text-amber-400"
            else:
                color = "text-red-400"
            latency_str = f'<span class="{color}">{lat}us</span>'
        else:
            latency_str = "-"

        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{html.escape(str(o.get("cid", "-")))}</td>'
            f'<td {_TD}>{html.escape(str(sym_name))}</td>'
            f'<td {_TD}>{html.escape(str(o.get("side", "-")))}</td>'
            f'<td {_TD}>{html.escape(str(o.get("price", "-")))}</td>'
            f'<td {_TD}>{html.escape(str(o.get("qty", "-")))}</td>'
            f'<td {_TD}>{html.escape(str(tif))}</td>'
            f'<td {_TD}>{html.escape(flags.strip())}</td>'
            f'<td {_TD}>{html.escape(str(o.get("status", "-")))}</td>'
            f'<td {_TD}>{latency_str}</td>'
            f'<td {_TD}>{html.escape(str(o.get("ts", "-")))}</td>'
            f'<td {_TD}>{cancel}</td></tr>'
        )
    return _table(
        ["CID", "Symbol", "Side", "Price", "Qty",
         "TIF", "Flags", "Status", "Latency", "Time", ""],
        rows,
    )


# ── Risk renderers ───────────────────────────────────────

def render_risk_user(data):
    """Render risk user data from Postgres."""
    if not data:
        return ('<span class="text-slate-600">'
                'no data found</span>')
    rows = ""
    for key, val in data.items():
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD} class="text-slate-500">'
            f'{html.escape(str(key))}</td>'
            f'<td {_TD}>{html.escape(str(val))}</td></tr>'
        )
    return _table(["Field", "Value"], rows)


def render_risk_user_wal(user_id: int, fills: list):
    """Render net positions for a user from WAL fill records."""
    positions: dict[int, dict] = {}
    for f in fills:
        sid = f.get("symbol_id", 0)
        qty = f.get("qty", 0)
        side = f.get("taker_side", 0)
        is_taker = f.get("taker_uid") == user_id
        # taker buys: side=0 → long; taker sells: side=1 → short
        # maker is opposite side
        if is_taker:
            signed = qty if side == 0 else -qty
        else:
            signed = -qty if side == 0 else qty
        entry = positions.setdefault(
            sid, {"net": 0, "fills": 0}
        )
        entry["net"] += signed
        entry["fills"] += 1
    if not positions:
        return ('<span class="text-slate-500 text-xs">'
                f'user {user_id} — no fills in WAL</span>')
    rows = ""
    for sid, info in sorted(positions.items()):
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        net = info["net"]
        n = info["fills"]
        net_str = format_qty(abs(net), sid)
        if net > 0:
            color = "text-emerald-400"
            label = f"+{net_str}"
        elif net < 0:
            color = "text-red-400"
            label = f"-{net_str}"
        else:
            color = "text-slate-500"
            label = "0"
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}>'
            f'<span class="{color}">{label}</span></td>'
            f'<td {_TD} class="text-slate-500">{n}</td>'
            f'</tr>'
        )
    return _table(
        ["Symbol", "Net (WAL)", "Fills"], rows,
    )


def render_liquidations_wal(records: list):
    """Render liquidation records from WAL."""
    if not records:
        return ('<span class="text-slate-600">'
                'no active liquidations</span>')
    rows = ""
    for r in records[:20]:
        sid = r.get("symbol_id", 0)
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        uid = r.get("user_id", 0)
        side = r.get("side", 0)
        qty = r.get("qty", 0)
        px = r.get("price", 0)
        slip = r.get("slip_bps", 0)
        side_str = (
            '<span class="text-emerald-400">buy</span>'
            if side == 0
            else '<span class="text-red-400">sell</span>'
        )
        qty_str = format_qty(abs(qty), sid)
        px_str = format_price(px, sid) if px else "--"
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{uid}</td>'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}>{side_str}</td>'
            f'<td {_TD}>{qty_str}</td>'
            f'<td {_TD}>{px_str}</td>'
            f'<td {_TD} class="text-slate-500">'
            f'{slip} bps</td></tr>'
        )
    return _table(
        ["User", "Symbol", "Side", "Qty", "Price",
         "Slip"], rows,
    )


# ── Risk dashboard panels ─────────────────────────────────

def _pct_bar(pct, color="blue"):
    """Horizontal percentage bar, 0-100."""
    clamped = max(0, min(100, pct))
    bar_cls = {
        "blue": "bg-blue-600",
        "amber": "bg-amber-500",
        "red": "bg-red-600",
        "emerald": "bg-emerald-600",
    }.get(color, "bg-blue-600")
    return (
        f'<div class="w-full bg-slate-800 rounded h-1.5">'
        f'<div class="{bar_cls} h-1.5 rounded"'
        f' style="width:{clamped:.1f}%"></div></div>'
        f'<span class="text-[10px] text-slate-500">'
        f'{clamped:.0f}%</span>'
    )


def _margin_gauge(ratio):
    """Colored margin ratio label + mini bar."""
    if ratio >= 999:
        return (
            '<span class="text-slate-500 font-mono">n/a</span>'
        )
    color = (
        "text-emerald-400" if ratio >= 2.0
        else "text-amber-400" if ratio >= 1.2
        else "text-red-400"
    )
    bar_color = (
        "emerald" if ratio >= 2.0
        else "amber" if ratio >= 1.2
        else "red"
    )
    bar_pct = min(100, ratio * 20)
    return (
        f'<span class="{color} font-mono">{ratio:.2f}x</span>'
        + _pct_bar(bar_pct, bar_color)
    )


def render_risk_overview(data, funding_data,
                         liq_data, insurance_data):
    """Render all 7 risk dashboard panels."""
    sim = data.get("simulated", True)
    sim_tag = (
        '<span class="text-[10px] text-slate-600 ml-1">'
        '(simulated)</span>'
        if sim else ''
    )
    system = data.get("system", {})
    users = data.get("users", [])

    # Panel 7: system-wide metrics
    oi = system.get("total_oi", 0)
    long_n = system.get("long_notional", 0)
    short_n = system.get("short_notional", 0)
    accts_pos = system.get("accounts_with_positions", 0)
    accts_liq = system.get("accounts_near_liq", 0)
    sys_panel = (
        '<div class="grid grid-cols-2 sm:grid-cols-5 gap-3">'
        + _metric("Total OI",
                  f"{oi // 10**8:,}" if oi else "--",
                  "blue-400")
        + _metric("Long Notional",
                  f"{long_n // 10**8:,}" if long_n else "--",
                  "emerald-400")
        + _metric("Short Notional",
                  f"{short_n // 10**8:,}"
                  if short_n else "--", "red-400")
        + _metric("Accounts w/ Positions",
                  str(accts_pos), "slate-300")
        + _metric("Near Liquidation", str(accts_liq),
                  "red-400" if accts_liq > 0 else "slate-500")
        + '</div>' + sim_tag
    )

    # Panels 1+2+3: per-user account, positions, margin
    user_cards = ""
    for u in users:
        uid = u["user_id"]
        collateral = u["collateral"]
        frozen = u["frozen"]
        available = u["available"]
        equity = u["equity"]
        upnl = u["upnl"]
        im_req = u["im_required"]
        mm_req = u["mm_required"]
        ratio = u["margin_ratio"]
        positions = u["positions"]

        util_pct = (
            frozen * 100 / collateral
            if collateral > 0 else 0.0
        )
        util_color = (
            "red" if util_pct > 80
            else "amber" if util_pct > 50
            else "blue"
        )
        upnl_color = (
            "text-emerald-400" if upnl >= 0
            else "text-red-400"
        )
        upnl_sign = "+" if upnl >= 0 else "-"
        upnl_str = format_price(abs(upnl), 1)

        acct_strip = (
            '<div class="grid grid-cols-2 sm:grid-cols-4'
            ' gap-2 mb-2">'
            + _metric("Collateral",
                      format_price(collateral, 1),
                      "slate-300")
            + _metric("Available",
                      format_price(available, 1),
                      "emerald-400"
                      if available > 0 else "red-400")
            + _metric("Equity",
                      format_price(abs(equity), 1),
                      "slate-300")
            + _metric("uPnL",
                      f"{upnl_sign}{upnl_str}",
                      upnl_color.replace("text-", ""))
            + '</div>'
        )
        margin_strip = (
            '<div class="flex flex-wrap items-center'
            ' gap-3 mb-2">'
            '<span class="text-[10px] text-slate-500">'
            'Margin util</span>'
            + _pct_bar(util_pct, util_color)
            + '<span class="text-[10px] text-slate-500'
            ' ml-2">Ratio&nbsp;'
            + _margin_gauge(ratio)
            + '</span></div>'
        )

        if positions:
            pos_rows = ""
            for pos in positions:
                sid = pos["symbol_id"]
                sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
                net = pos["net"]
                entry_px = pos["entry_px"]
                mark_px = pos["mark_px"]
                pnl = pos["upnl"]
                notional = pos["notional"]
                im = pos["im"]
                mm = pos["mm"]
                pos_sim = pos.get("simulated", False)
                side_str = (
                    '<span class="text-emerald-400">'
                    'long</span>'
                    if net > 0
                    else '<span class="text-red-400">'
                    'short</span>'
                )
                pnl_color = (
                    "text-emerald-400" if pnl >= 0
                    else "text-red-400"
                )
                pnl_sign = "+" if pnl >= 0 else "-"
                pnl_str = format_price(abs(pnl), sid)
                row_cls = (
                    "bg-red-900/20 hover:bg-red-900/30"
                    if ratio < 1.2
                    else (
                        "bg-amber-900/20"
                        " hover:bg-amber-900/30"
                        if ratio < 1.5
                        else "hover:bg-slate-800/50"
                    )
                )
                sim_dot = (
                    '<span class="text-[9px]'
                    ' text-slate-600 ml-1">~</span>'
                    if pos_sim else ''
                )
                pos_rows += (
                    f'<tr class="{row_cls}">'
                    f'<td {_TD}>{sym}{sim_dot}</td>'
                    f'<td {_TD}>{side_str}</td>'
                    f'<td {_TD}>'
                    f'{format_qty(abs(net), sid)}</td>'
                    f'<td {_TD}>'
                    f'{format_price(entry_px, sid)}</td>'
                    f'<td {_TD}>'
                    f'{format_price(mark_px, sid)}</td>'
                    f'<td {_TD}>'
                    f'<span class="{pnl_color}">'
                    f'{pnl_sign}{pnl_str}</span></td>'
                    f'<td {_TD}>'
                    f'{format_price(notional, sid)}</td>'
                    f'<td {_TD} class="text-slate-500">'
                    f'{format_price(im, sid)}</td>'
                    f'<td {_TD} class="text-slate-500">'
                    f'{format_price(mm, sid)}</td>'
                    f'</tr>'
                )
            pos_table = _table(
                ["Symbol", "Side", "Qty", "Entry",
                 "Mark", "uPnL", "Notional", "IM", "MM"],
                pos_rows,
            )
        else:
            pos_table = (
                '<span class="text-slate-600 text-xs">'
                'no open positions</span>'
            )

        margin_summary = (
            '<div class="grid grid-cols-3 gap-2 mt-2">'
            + _metric("IM Required",
                      format_price(im_req, 1), "slate-400")
            + _metric("MM Required",
                      format_price(mm_req, 1), "slate-400")
            + _metric("Margin Ratio",
                      f"{ratio:.2f}x" if ratio < 999 else "n/a",
                      "emerald-400" if ratio >= 2.0
                      else "amber-400" if ratio >= 1.2
                      else "red-400")
            + '</div>'
        )

        user_cards += (
            f'<div class="bg-slate-800/40 border'
            f' border-slate-700/50 rounded p-3 mb-2">'
            f'<div class="text-xs font-semibold'
            f' text-slate-400 mb-2">user {uid}</div>'
            f'{acct_strip}'
            f'{margin_strip}'
            f'{pos_table}'
            f'{margin_summary}'
            f'</div>'
        )

    if not user_cards:
        user_cards = (
            '<span class="text-slate-600 text-xs">'
            'no account data — start RSX or run '
            'a load test</span>'
        )

    # Panel 4: funding rates
    fund_rows = ""
    for fe in funding_data.get("funding", []):
        sid = fe["symbol_id"]
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        rate = fe["rate_bps"]
        mark = fe["mark_px"]
        idx = fe["index_px"]
        prem = fe["premium_bps"]
        nxt = fe["next_settlement_s"]
        h = nxt // 3600
        m = (nxt % 3600) // 60
        countdown = f"{h}h{m:02d}m"
        rate_color = (
            "text-red-400" if rate > 5
            else "text-emerald-400" if rate < 0
            else "text-slate-300"
        )
        fund_rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}><span class="{rate_color}">'
            f'{rate:+d} bps</span></td>'
            f'<td {_TD}>{countdown}</td>'
            f'<td {_TD}>'
            f'{format_price(mark, sid)}</td>'
            f'<td {_TD}>'
            f'{format_price(idx, sid)}</td>'
            f'<td {_TD} class="text-slate-500">'
            f'{prem:+d} bps</td>'
            f'</tr>'
        )
    fund_panel = (
        _table(
            ["Symbol", "Rate", "Next", "Mark",
             "Index", "Premium"],
            fund_rows,
        )
        if fund_rows
        else (
            '<span class="text-slate-500 text-xs">'
            'no BBO data</span>'
        )
    )

    # Panel 5: liquidation queue
    liq_records = liq_data.get("liquidations", [])
    if liq_records:
        liq_rows = ""
        for r in liq_records[:15]:
            sid = r.get("symbol_id", 0)
            uid2 = r.get("user_id", 0)
            sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
            slip = r.get("slip_bps", 0)
            rnd = r.get("round", 0)
            status = r.get("status", 0)
            status_str = (
                '<span class="text-red-400">active</span>'
                if status == 0
                else '<span class="text-slate-500">'
                'halted</span>'
            )
            liq_rows += (
                f'<tr class="hover:bg-slate-800/50">'
                f'<td {_TD}>{uid2}</td>'
                f'<td {_TD}>{sym}</td>'
                f'<td {_TD}>{status_str}</td>'
                f'<td {_TD} class="text-slate-500">'
                f'{rnd}</td>'
                f'<td {_TD} class="text-slate-500">'
                f'{slip} bps</td>'
                f'</tr>'
            )
        liq_panel = _table(
            ["User", "Symbol", "Status", "Round", "Slip"],
            liq_rows,
        )
    else:
        liq_panel = (
            '<span class="text-slate-500 text-xs">'
            'no active liquidations</span>'
        )

    # Panel 6: insurance fund
    funds = insurance_data.get("funds", [])
    ins_source = insurance_data.get("source", "simulated")
    ins_sim = ins_source == "simulated"
    total_ins = insurance_data.get("total", 0)
    if funds:
        ins_rows = ""
        for fi in funds:
            sid = fi.get("symbol_id", 0)
            sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
            bal = fi.get("balance", 0)
            ver = fi.get("version", 0)
            ins_rows += (
                f'<tr class="hover:bg-slate-800/50">'
                f'<td {_TD}>{sym}</td>'
                f'<td {_TD} class="text-emerald-400">'
                f'{format_price(bal, sid)}</td>'
                f'<td {_TD} class="text-slate-500">'
                f'{ver}</td>'
                f'</tr>'
            )
        ins_sim_note = (
            '<span class="text-[10px] text-slate-600 ml-1">'
            '(simulated)</span>'
            if ins_sim else ''
        )
        ins_panel = (
            _table(["Symbol", "Balance", "Version"], ins_rows)
            + f'<div class="mt-2 text-xs text-slate-400">'
            f'total: <span class="text-emerald-400">'
            f'{format_price(total_ins, 1)}</span>'
            + ins_sim_note
            + '</div>'
        )
    else:
        ins_panel = (
            '<span class="text-slate-500 text-xs">'
            'no insurance fund data</span>'
        )

    return f"""
<div class="space-y-4">
<div>
  <h3 class="text-[10px] font-semibold text-slate-500
    uppercase tracking-wider mb-2">
    System-wide Risk Metrics</h3>
  {sys_panel}
</div>
<div>
  <h3 class="text-[10px] font-semibold text-slate-500
    uppercase tracking-wider mb-2">
    Account Overview &amp; Open Positions</h3>
  {user_cards}
</div>
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
  <div>
    <h3 class="text-[10px] font-semibold text-slate-500
      uppercase tracking-wider mb-2">Funding Rates</h3>
    {fund_panel}
  </div>
  <div>
    <h3 class="text-[10px] font-semibold text-slate-500
      uppercase tracking-wider mb-2">Insurance Fund</h3>
    {ins_panel}
  </div>
</div>
<div>
  <h3 class="text-[10px] font-semibold text-slate-500
    uppercase tracking-wider mb-2">Liquidation Queue</h3>
  {liq_panel}
</div>
</div>"""


def render_position_heatmap(fills=None):
    """Position heatmap from WAL fill data."""
    if not fills:
        return ('<span class="text-slate-500 text-xs">'
                'no fill data available</span>')
    # Aggregate net position per symbol_id from fills
    positions: dict[int, int] = {}
    for f in fills:
        sid = f.get("symbol_id", 0)
        qty = f.get("qty", 0)
        side = f.get("taker_side", 0)
        signed = qty if side == 0 else -qty
        positions[sid] = positions.get(sid, 0) + signed
    rows = ""
    for sid, net in sorted(
        positions.items(),
        key=lambda kv: SYMBOL_NAMES.get(kv[0], f"sym-{kv[0]}"),
    ):
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        net_str = format_qty(abs(net), sid)
        abs_str = format_qty(abs(net), sid)
        if net > 0:
            color = "text-emerald-400"
            label = f"+{net_str}"
        elif net < 0:
            color = "text-red-400"
            label = f"-{net_str}"
        else:
            color = "text-slate-500"
            label = "0"
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}><span class="{color}">'
            f'{label}</span></td>'
            f'<td {_TD}>{abs_str}</td></tr>'
        )
    return _table(
        ["Symbol", "Net Position", "Abs Size"], rows,
    )


def render_margin_ladder(fills=None):
    """Margin ladder from WAL fill data."""
    if not fills:
        return ('<span class="text-slate-500 text-xs">'
                'no fill data available</span>')
    # Show recent fills as proxy for margin impact
    rows = ""
    for f in fills[:20]:
        sid = f.get("symbol_id", 0)
        sym = {1: "BTC", 2: "ETH", 3: "SOL",
               10: "PENGU"}.get(sid, f"sym-{sid}")
        px = f.get("price", 0)
        qty = f.get("qty", 0)
        side = f.get("taker_side", 0)
        notional = abs(px * qty)
        px_str = format_price(px, sid)
        qty_str = format_qty(qty, sid)
        notional_str = format_price(notional, sid)
        side_str = ('<span class="text-emerald-400">'
                    'buy</span>' if side == 0
                    else '<span class="text-red-400">'
                    'sell</span>')
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}>{side_str}</td>'
            f'<td {_TD}>{px_str}</td>'
            f'<td {_TD}>{qty_str}</td>'
            f'<td {_TD}>{notional_str}</td></tr>'
        )
    return _table(
        ["Symbol", "Side", "Price", "Qty", "Notional"],
        rows,
    )


def render_funding(book_stats=None):
    """Funding tracking from WAL BBO data."""
    if not book_stats:
        return ('<span class="text-slate-500 text-xs">'
                'no BBO data available</span>')
    rows = ""
    for sid, bbo in sorted(book_stats.items()):
        sym = {1: "BTC", 2: "ETH", 3: "SOL",
               10: "PENGU"}.get(sid, f"sym-{sid}")
        bid = bbo.get("bid_px", 0)
        ask = bbo.get("ask_px", 0)
        mid = (bid + ask) // 2 if bid and ask else 0
        spread = ask - bid if bid and ask else 0
        # Simplified funding = spread / mid basis points
        rate = (
            f"{spread * 10000 // mid} bps"
            if mid > 0 else "--"
        )
        bid_str = format_price(bid, sid) if bid else "--"
        ask_str = format_price(ask, sid) if ask else "--"
        spread_str = format_price(spread, sid) if spread else "--"
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{sym}</td>'
            f'<td {_TD}>{bid_str}</td>'
            f'<td {_TD}>{ask_str}</td>'
            f'<td {_TD}>{spread_str}</td>'
            f'<td {_TD}>{rate}</td></tr>'
        )
    return _table(
        ["Symbol", "Bid", "Ask", "Spread", "Rate"], rows,
    )


def render_risk_latency(latencies=None):
    """Display order latency percentiles from recent submissions."""
    if not latencies or len(latencies) == 0:
        return (
            '<div class="flex gap-6">'
            + _metric("p50", "--", "slate-500")
            + _metric("p95", "--", "slate-500")
            + _metric("p99", "--", "slate-500")
            + _metric("max", "--", "slate-500")
            + '</div>'
        )

    sorted_lat = sorted(latencies)
    p50 = int(_percentile(sorted_lat, 50))
    p95 = int(_percentile(sorted_lat, 95))
    p99 = int(_percentile(sorted_lat, 99))
    max_lat = sorted_lat[-1]

    def color_for_lat(lat_us):
        if lat_us < 100:
            return "emerald-400"
        elif lat_us < 500:
            return "amber-400"
        else:
            return "red-400"

    return (
        '<div class="flex gap-6">'
        + _metric("p50", f"{p50}us", color_for_lat(p50))
        + _metric("p95", f"{p95}us", color_for_lat(p95))
        + _metric("p99", f"{p99}us", color_for_lat(p99))
        + _metric("max", f"{max_lat}us", color_for_lat(max_lat))
        + f'<div class="text-xs text-slate-500 self-center">n={len(latencies)}</div>'
        + '</div>'
    )


SYMBOL_NAMES = {
    1: "BTC", 2: "ETH", 3: "SOL", 10: "PENGU",
}

# Symbol display config (for formatting WAL data)
# tick_size=1, lot_size=1 in test env, but values may be large
SYMBOL_CONFIG = {
    1: {"price_decimals": 2, "qty_decimals": 8},    # BTC
    2: {"price_decimals": 2, "qty_decimals": 6},    # ETH
    3: {"price_decimals": 4, "qty_decimals": 4},    # SOL
    10: {"price_decimals": 6, "qty_decimals": 2},   # PENGU
}


def format_price(raw_price, symbol_id):
    """Format raw i64 price for display."""
    if raw_price == 0:
        return "0"
    cfg = SYMBOL_CONFIG.get(symbol_id, {"price_decimals": 2})
    decimals = cfg["price_decimals"]
    # For tick_size=1, display as-is with decimal separator
    if decimals == 0:
        return str(raw_price)
    # Add decimal point
    sign = "-" if raw_price < 0 else ""
    abs_val = abs(raw_price)
    scale = 10 ** decimals
    whole = abs_val // scale
    frac = abs_val % scale
    return f"{sign}{whole}.{frac:0{decimals}d}".rstrip('0').rstrip('.')


def format_qty(raw_qty, symbol_id):
    """Format raw i64 qty for display."""
    if raw_qty == 0:
        return "0"
    cfg = SYMBOL_CONFIG.get(symbol_id, {"qty_decimals": 2})
    decimals = cfg["qty_decimals"]
    if decimals == 0:
        return str(raw_qty)
    sign = "-" if raw_qty < 0 else ""
    abs_val = abs(raw_qty)
    scale = 10 ** decimals
    whole = abs_val // scale
    frac = abs_val % scale
    return f"{sign}{whole}.{frac:0{decimals}d}".rstrip('0').rstrip('.')


def render_book_ladder(symbol_id, snap):
    """Render orderbook ladder from depth snap.

    snap: {"bids": [{"px": int, "qty": int}, ...],
           "asks": [{"px": int, "qty": int}, ...]}
    Asks shown top (worst→best price), bids below.
    """
    sym = SYMBOL_NAMES.get(symbol_id, f"sym-{symbol_id}")
    if snap is None:
        return (
            f'<div class="text-slate-500 text-xs">'
            f'{sym}: no book data yet (waiting for orders)'
            f'</div>')
    bids = snap.get("bids", [])
    asks = snap.get("asks", [])
    if not bids and not asks:
        return (
            f'<div class="text-slate-500 text-xs">'
            f'{sym}: no book data yet (waiting for orders)'
            f'</div>')
    best_bid = bids[0].get("px", 0) if bids else 0
    best_ask = asks[0].get("px", 0) if asks else 0
    spread = (
        best_ask - best_bid
        if best_ask > 0 and best_bid > 0
        else 0
    )
    ask_rows = ""
    for lvl in reversed(asks[:10]):
        px = lvl.get("px", 0)
        qty = lvl.get("qty", 0)
        ask_rows += (
            f'<tr class="text-red-400"'
            f' data-testid="ask-row" data-px="{px}">'
            f'<td>Ask</td>'
            f'<td class="text-right font-mono">{px}</td>'
            f'<td class="text-right font-mono">{qty}</td>'
            f'</tr>\n'
        )
    bid_rows = ""
    for lvl in bids[:10]:
        px = lvl.get("px", 0)
        qty = lvl.get("qty", 0)
        bid_rows += (
            f'<tr class="text-emerald-400"'
            f' data-testid="bid-row" data-px="{px}">'
            f'<td>Bid</td>'
            f'<td class="text-right font-mono">{px}</td>'
            f'<td class="text-right font-mono">{qty}</td>'
            f'</tr>\n'
        )
    return f"""<table class="w-full text-xs">
<tr class="text-slate-500">
  <th class="text-left">Side</th>
  <th class="text-right">Price</th>
  <th class="text-right">Qty</th>
</tr>
{ask_rows}<tr class="text-slate-600 text-center">
  <td colspan="3">spread: {spread}</td>
</tr>
{bid_rows}</table>"""


def render_book_stats(symbols):
    """Render book stats from BBO records per symbol."""
    if not symbols:
        return (
            '<span class="text-slate-500 text-xs">'
            'no book data yet (waiting for orders)</span>')
    rows = []
    for sid, bbo in sorted(symbols.items()):
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        bid_raw = bbo.get("bid_px", 0)
        ask_raw = bbo.get("ask_px", 0)
        spread_raw = ask_raw - bid_raw if (
            ask_raw > 0 and bid_raw > 0
        ) else 0
        bid_fmt = format_price(bid_raw, sid)
        ask_fmt = format_price(ask_raw, sid)
        spread_fmt = format_price(spread_raw, sid) if spread_raw > 0 else "0"
        orders = (
            bbo.get("bid_count", 0) + bbo.get("ask_count", 0)
        )
        rows.append(
            f'<tr><td>{sym}</td>'
            f'<td class="text-right font-mono">'
            f'{bid_fmt}</td>'
            f'<td class="text-right font-mono">'
            f'{ask_fmt}</td>'
            f'<td class="text-right">{spread_fmt}</td>'
            f'<td class="text-right">'
            f'{orders}</td>'
            f'</tr>')
    return f"""
<table class="w-full text-xs">
<tr class="text-slate-500">
  <th class="text-left">Symbol</th>
  <th class="text-right">Bid</th>
  <th class="text-right">Ask</th>
  <th class="text-right">Spread</th>
  <th class="text-right">Orders</th>
</tr>
{''.join(rows)}
</table>"""


def render_live_fills(fills):
    """Render recent fills list."""
    if not fills:
        return (
            '<span class="text-slate-500 text-xs">'
            'no fills yet</span>')
    rows = []
    for f in fills[:20]:
        sid = f.get("symbol_id", 0)
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        side = "buy" if f.get("taker_side", 0) == 0 else "sell"
        row_cls = "text-emerald-400" if side == "buy" else "text-red-400"
        price_fmt = format_price(f.get("price", 0), sid)
        qty_fmt = format_qty(f.get("qty", 0), sid)
        rows.append(
            f'<tr class="{row_cls}">'
            f'<td>{sym}</td>'
            f'<td>{side}</td>'
            f'<td class="text-right font-mono">'
            f'{price_fmt}</td>'
            f'<td class="text-right font-mono">'
            f'{qty_fmt}</td>'
            f'<td class="text-slate-500">'
            f'{f.get("seq", 0)}</td></tr>')
    return f"""
<table class="w-full text-xs">
<tr class="text-slate-500">
  <th class="text-left">Symbol</th>
  <th>Side</th>
  <th class="text-right">Price</th>
  <th class="text-right">Qty</th>
  <th>Seq</th>
</tr>
{''.join(rows)}
</table>"""


def render_order_trace(order, fills):
    """Render lifecycle trace for a single order."""
    cid = html.escape(order.get("cid", ""))
    sym_id = order.get("symbol", "10")
    sym = SYMBOL_NAMES_STR.get(sym_id, f"sym-{sym_id}")
    side = html.escape(order.get("side", "buy"))
    price = html.escape(str(order.get("price", "-")))
    qty = html.escape(str(order.get("qty", "-")))
    status = order.get("status", "pending")
    ts = html.escape(order.get("ts", "-"))

    _STEP_BG = {
        "blue": "bg-blue-400",
        "emerald": "bg-emerald-400",
        "amber": "bg-amber-400",
        "red": "bg-red-400",
        "slate": "bg-slate-400",
    }
    _STEP_TEXT = {
        "blue": "text-blue-400",
        "emerald": "text-emerald-400",
        "amber": "text-amber-400",
        "red": "text-red-400",
        "slate": "text-slate-400",
    }

    def step(color, label, detail=""):
        detail_html = (
            f' <span class="text-slate-500">{detail}</span>'
            if detail else ""
        )
        bg = _STEP_BG.get(color, "bg-slate-400")
        txt = _STEP_TEXT.get(color, "text-slate-400")
        return (
            f'<div class="flex items-start gap-2 py-1">'
            f'<div class="mt-0.5 w-2 h-2 rounded-full '
            f'{bg} shrink-0"></div>'
            f'<span class="{txt} text-xs">'
            f'{label}</span>'
            f'{detail_html}</div>'
        )

    steps = [
        step("blue", "submitted",
             f"{sym} {side} {qty}@{price} at {ts}"),
    ]

    if status == "accepted":
        steps.append(
            step("emerald", "routed",
                 f"gateway accepted ({order.get('latency_us', '-')}us)")
        )
        if fills:
            fill_count = len(fills)
            fill_qty = sum(f.get("qty", 0) for f in fills)
            steps.append(
                step("emerald", "filled",
                     f"{fill_count} fill(s), total qty {fill_qty}")
            )
        else:
            steps.append(step("amber", "open", "resting in book"))
    elif status == "rejected":
        reason = html.escape(order.get("reason", "unknown"))
        steps.append(step("red", "rejected", reason))
    elif status == "error":
        err = html.escape(order.get("error", "unknown"))
        steps.append(step("red", "error", err))
    else:
        steps.append(step("slate", "pending", "awaiting gateway"))

    inner = "".join(steps)
    return (
        f'<div class="font-mono text-xs text-slate-400 mb-1">'
        f'oid: {cid}</div>'
        f'<div class="border-l-2 border-slate-700 pl-3">'
        f'{inner}</div>'
    )


def render_trade_agg(fills):
    """Render trade aggregation from fills."""
    if not fills:
        return (
            '<span class="text-slate-500 text-xs">'
            'no trade data yet</span>')
    by_sym = {}
    for f in fills:
        sid = f.get("symbol_id", 0)
        if sid not in by_sym:
            by_sym[sid] = {
                "count": 0, "volume": 0,
                "last_px": 0}
        by_sym[sid]["count"] += 1
        by_sym[sid]["volume"] += abs(f.get("qty", 0))
        by_sym[sid]["last_px"] = f.get("price", 0)
    rows = []
    for sid, agg in sorted(by_sym.items()):
        sym = SYMBOL_NAMES.get(sid, f"sym-{sid}")
        rows.append(
            f'<tr><td>{sym}</td>'
            f'<td class="text-right">{agg["count"]}</td>'
            f'<td class="text-right font-mono">'
            f'{agg["volume"]}</td>'
            f'<td class="text-right font-mono">'
            f'{agg["last_px"]}</td></tr>')
    return f"""
<table class="w-full text-xs">
<tr class="text-slate-500">
  <th class="text-left">Symbol</th>
  <th class="text-right">Fills</th>
  <th class="text-right">Volume</th>
  <th class="text-right">Last Px</th>
</tr>
{''.join(rows)}
</table>"""


def render_stress_scenarios(scenario_states: dict) -> str:
    """HTMX partial: scenario toggle panel."""
    rows = ""
    for name, state in scenario_states.items():
        running = state["running"]
        desc = html.escape(state["desc"])
        esc_name = html.escape(name)
        dot = (
            '<span class="text-emerald-400">&#9679;</span>'
            if running
            else '<span class="text-zinc-600">&#9675;</span>'
        )
        if running:
            btn = (
                f'<button'
                f' hx-post="./api/stress/scenario/{esc_name}/stop"'
                f' hx-target="#stress-scenarios"'
                f' hx-swap="outerHTML"'
                f' class="px-2 py-0.5 rounded text-[10px]'
                f' bg-amber-900/40 text-amber-400'
                f' border border-amber-800'
                f' hover:bg-amber-800 cursor-pointer">'
                f'&#9632; stop</button>'
            )
        else:
            btn = (
                f'<button'
                f' hx-post="./api/stress/scenario/{esc_name}/start"'
                f' hx-target="#stress-scenarios"'
                f' hx-swap="outerHTML"'
                f' class="px-2 py-0.5 rounded text-[10px]'
                f' bg-blue-900/40 text-blue-400'
                f' border border-blue-800'
                f' hover:bg-blue-800 cursor-pointer">'
                f'&#9654; start</button>'
            )
        rows += (
            f'<div class="flex items-center gap-3 py-1'
            f' border-b border-slate-800/50 last:border-0">'
            f'<span class="w-4 text-center">{dot}</span>'
            f'<span class="w-32 text-xs font-mono'
            f' text-slate-300">{esc_name}</span>'
            f'<span class="flex-1 text-xs text-slate-500'
            f' truncate">{desc}</span>'
            f'{btn}'
            f'</div>'
        )
    return (
        f'<div id="stress-scenarios"'
        f' class="bg-slate-900 border border-slate-800'
        f' rounded-lg p-4"'
        f' hx-get="./x/stress-scenarios"'
        f' hx-trigger="every 3s"'
        f' hx-swap="outerHTML">'
        f'<div class="flex items-center justify-between mb-3">'
        f'<h2 class="text-xs font-semibold text-slate-500'
        f' uppercase tracking-wider">Stress Scenarios</h2>'
        f'</div>'
        f'{rows}'
        f'</div>'
    )


def stress_page():
    """Stress test page with launcher and reports list."""
    launcher = _card(
        "Run Stress Test",
        """
<form hx-post="./api/stress/run" hx-target="#stress-result" hx-swap="innerHTML">
  <div class="space-y-3">
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label class="block text-xs text-slate-400 mb-1">Rate (orders/sec)</label>
        <input type="number" name="rate" value="100" min="10" max="10000"
          class="w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-xs" />
      </div>
      <div>
        <label class="block text-xs text-slate-400 mb-1">Duration (seconds)</label>
        <input type="number" name="duration" value="60" min="1" max="600"
          class="w-full bg-slate-900 border border-slate-700 rounded px-2 py-1 text-xs" />
      </div>
    </div>
    <div class="flex gap-2">
      <button type="submit"
        class="bg-blue-900/60 text-blue-400 px-4 py-2 rounded text-xs
          border border-blue-800 hover:bg-blue-800 cursor-pointer">
        Run Stress Test
      </button>
      <span class="htmx-indicator text-amber-400 text-xs self-center">Running...</span>
    </div>
    <div id="stress-result" class="text-xs"></div>
  </div>
</form>
        """,
    )

    reports_list = _card(
        "Historical Reports",
        """
<div hx-get="./x/stress-reports-list"
     hx-trigger="load, every 5s"
     hx-swap="innerHTML">
  Loading...
</div>
        """,
    )

    info = _card(
        "About Stress Testing",
        """
<div class="space-y-2 text-xs text-slate-400">
  <p>Stress tests submit orders to the Gateway WebSocket at a specified rate and measure:</p>
  <ul class="list-disc list-inside space-y-1 ml-2">
    <li><strong>Throughput:</strong> Actual orders/sec achieved vs target</li>
    <li><strong>Latency:</strong> p50/p95/p99/max round-trip time (submit → ack)</li>
    <li><strong>Acceptance Rate:</strong> % of orders accepted by risk engine</li>
    <li><strong>Errors:</strong> Connection failures, timeouts, gateway overload</li>
  </ul>
  <p class="mt-3">Click on a report timestamp to view detailed results with charts.</p>
</div>
        """,
    )

    scenarios_panel = (
        '<div id="stress-scenarios"'
        ' class="bg-slate-900 border border-slate-800'
        ' rounded-lg p-4"'
        ' hx-get="./x/stress-scenarios"'
        ' hx-trigger="load, every 3s"'
        ' hx-swap="outerHTML">'
        '<span class="text-slate-500 text-xs">Loading scenarios...</span>'
        '</div>'
    )

    content = f"""
{scenarios_panel}
{launcher}
{reports_list}
{info}
    """

    return layout("Stress Testing", content, active_tab="./stress")


def stress_report_page(data):
    """Individual stress test report with charts and details."""
    config = data.get("config", {})
    metrics = data.get("metrics", {})
    latency = data.get("latency_us", {})
    timestamp = data.get("timestamp", "unknown")

    # Format timestamp
    if len(timestamp) == 15:
        t = timestamp
        ts_fmt = (f"{t[0:4]}-{t[4:6]}-{t[6:8]}"
                  f" {t[9:11]}:{t[11:13]}:{t[13:15]}")
    else:
        ts_fmt = timestamp

    # Summary card
    summary = _card(
        f"Stress Test Report: {ts_fmt}",
        f"""
<div class="grid grid-cols-2 md:grid-cols-4 gap-4">
  <div>
    <div class="text-[10px] text-slate-500 uppercase">Target Rate</div>
    <div class="text-lg text-blue-400">{config.get("target_rate", 0):,}/s</div>
  </div>
  <div>
    <div class="text-[10px] text-slate-500 uppercase">Duration</div>
    <div class="text-lg text-blue-400">{config.get("duration", 0)}s</div>
  </div>
  <div>
    <div class="text-[10px] text-slate-500 uppercase">Actual Rate</div>
    <div class="text-lg text-emerald-400">{metrics.get("actual_rate", 0):,.1f}/s</div>
  </div>
  <div>
    <div class="text-[10px] text-slate-500 uppercase">Elapsed</div>
    <div class="text-lg text-slate-400">{metrics.get("elapsed_sec", 0):.2f}s</div>
  </div>
</div>
        """,
    )

    # Results card
    submitted = metrics.get("submitted", 0)
    accepted = metrics.get("accepted", 0)
    rejected = metrics.get("rejected", 0)
    errors = metrics.get("errors", 0)
    accept_rate = metrics.get("accept_rate", 0)

    if accept_rate >= 95:
        accept_color = "text-emerald-400"
        accept_bg = "bg-emerald-400"
    elif accept_rate >= 90:
        accept_color = "text-amber-400"
        accept_bg = "bg-amber-400"
    else:
        accept_color = "text-red-400"
        accept_bg = "bg-red-400"

    results = _card(
        "Results",
        f"""
<div class="space-y-3">
  <div class="grid grid-cols-4 gap-4">
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Submitted</div>
      <div class="text-xl text-slate-300">{submitted:,}</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Accepted</div>
      <div class="text-xl {accept_color}">{accepted:,}</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Rejected</div>
      <div class="text-xl text-amber-400">{rejected:,}</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Errors</div>
      <div class="text-xl text-red-400">{errors:,}</div>
    </div>
  </div>
  <div class="flex gap-6 pt-3 border-t border-slate-800">
    <div class="flex-1">
      <div class="text-[10px] text-slate-500 uppercase mb-1">Accept Rate</div>
      <div class="flex items-center gap-2">
        <div class="flex-1 bg-slate-800 rounded h-2 overflow-hidden">
          <div class="{accept_bg} h-full" style="width: {accept_rate}%"></div>
        </div>
        <span class="text-xs {accept_color} w-12 text-right">{accept_rate}%</span>
      </div>
    </div>
  </div>
</div>
        """,
    )

    # Latency card
    p50 = latency.get("p50", 0)
    p95 = latency.get("p95", 0)
    p99 = latency.get("p99", 0)
    min_lat = latency.get("min", 0)
    max_lat = latency.get("max", 0)

    def latency_color(lat_us):
        if lat_us < 1000:
            return ("text-emerald-400", "bg-emerald-400")
        elif lat_us < 5000:
            return ("text-amber-400", "bg-amber-400")
        else:
            return ("text-red-400", "bg-red-400")

    lc_min = latency_color(min_lat)
    lc_p50 = latency_color(p50)
    lc_p95 = latency_color(p95)
    lc_p99 = latency_color(p99)
    lc_max = latency_color(max_lat)

    latency_card = _card(
        "Latency Distribution (microseconds)",
        f"""
<div class="space-y-4">
  <div class="grid grid-cols-5 gap-3">
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Min</div>
      <div class="text-lg {lc_min[0]}">{min_lat:,}µs</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">p50</div>
      <div class="text-lg {lc_p50[0]}">{p50:,}µs</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">p95</div>
      <div class="text-lg {lc_p95[0]}">{p95:,}µs</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">p99</div>
      <div class="text-lg {lc_p99[0]}">{p99:,}µs</div>
    </div>
    <div>
      <div class="text-[10px] text-slate-500 uppercase">Max</div>
      <div class="text-lg {lc_max[0]}">{max_lat:,}µs</div>
    </div>
  </div>
  <div class="space-y-2">
    <div class="flex items-center gap-2">
      <span class="text-xs text-slate-400 w-16">p50</span>
      <div class="flex-1 bg-slate-800 rounded h-2 overflow-hidden">
        <div class="{lc_p50[1]} h-full"
          style="width: {min(100, p50 * 100 / max(max_lat, 1)):.1f}%"></div>
      </div>
      <span class="text-xs {lc_p50[0]} w-20 text-right">{p50:,}µs</span>
    </div>
    <div class="flex items-center gap-2">
      <span class="text-xs text-slate-400 w-16">p95</span>
      <div class="flex-1 bg-slate-800 rounded h-2 overflow-hidden">
        <div class="{lc_p95[1]} h-full"
          style="width: {min(100, p95 * 100 / max(max_lat, 1)):.1f}%"></div>
      </div>
      <span class="text-xs {lc_p95[0]} w-20 text-right">{p95:,}µs</span>
    </div>
    <div class="flex items-center gap-2">
      <span class="text-xs text-slate-400 w-16">p99</span>
      <div class="flex-1 bg-slate-800 rounded h-2 overflow-hidden">
        <div class="{lc_p99[1]} h-full"
          style="width: {min(100, p99 * 100 / max(max_lat, 1)):.1f}%"></div>
      </div>
      <span class="text-xs {lc_p99[0]} w-20 text-right">{p99:,}µs</span>
    </div>
    <div class="flex items-center gap-2">
      <span class="text-xs text-slate-400 w-16">max</span>
      <div class="flex-1 bg-slate-800 rounded h-2 overflow-hidden">
        <div class="{lc_max[1]} h-full" style="width: 100%"></div>
      </div>
      <span class="text-xs {lc_max[0]} w-20 text-right">{max_lat:,}µs</span>
    </div>
  </div>
</div>
        """,
    )

    # Pass/Fail Assessment
    passed_rate = accept_rate >= 95
    passed_p99 = p99 < 10000  # 10ms
    passed_errors = (errors / max(submitted, 1) * 100) < 1

    status = "PASS" if (passed_rate and passed_p99 and passed_errors) else "FAIL"
    if status == "PASS":
        status_bg = "bg-emerald-900/40"
        status_border = "border-emerald-800"
        status_text = "text-emerald-400"
    else:
        status_bg = "bg-red-900/40"
        status_border = "border-red-800"
        status_text = "text-red-400"
    rate_cls = "text-emerald-400" if passed_rate else "text-red-400"
    rate_mark = "✓" if passed_rate else "✗"
    p99_cls = "text-emerald-400" if passed_p99 else "text-red-400"
    p99_mark = "✓" if passed_p99 else "✗"
    err_cls = "text-emerald-400" if passed_errors else "text-red-400"
    err_mark = "✓" if passed_errors else "✗"

    assessment = _card(
        "Assessment",
        f"""
<div class="space-y-3">
  <div class="text-center">
    <div class="inline-block {status_bg} border {status_border}
      {status_text} px-6 py-3 rounded-lg text-2xl font-bold">
      {status}
    </div>
  </div>
  <div class="space-y-2 text-xs">
    <div class="flex items-center gap-2">
      <span class="{rate_cls}">
        {rate_mark}
      </span>
      <span>Accept rate ≥95%: {accept_rate}%</span>
    </div>
    <div class="flex items-center gap-2">
      <span class="{p99_cls}">
        {p99_mark}
      </span>
      <span>p99 latency <10ms: {p99/1000:.2f}ms</span>
    </div>
    <div class="flex items-center gap-2">
      <span class="{err_cls}">
        {err_mark}
      </span>
      <span>Error rate <1%: {errors/max(submitted,1)*100:.2f}%</span>
    </div>
  </div>
</div>
        """,
    )

    nav = f"""
<div class="mb-4">
  <a href="../stress" class="text-blue-400 hover:underline text-xs">
    ← Back to Stress Tests
  </a>
</div>
    """

    content = f"""
{nav}
{summary}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
  {results}
  {assessment}
</div>
{latency_card}
    """

    return layout(f"Stress Report: {ts_fmt}", content, active_tab="./stress")


def maker_status_html(stats: dict, pid) -> str:
    """Render maker-status HTMX partial with live fields."""
    orders_placed = stats.get("orders_placed", "--")
    active_orders = stats.get("active_orders", "--")
    mid_prices = stats.get("mid_prices", {})
    mid_price = (
        next(iter(mid_prices.values()), "--")
        if mid_prices else "--"
    )
    return (
        f'<span class="text-emerald-400 text-xs">'
        f'running (pid {pid}) &mdash; '
        f'orders_placed: {orders_placed}, '
        f'active_orders: {active_orders}, '
        f'mid_price: {mid_price}'
        f'</span>'
    )


# ── Screen: Market Maker ─────────────────────────────────

def maker_live_html(
    running: bool,
    pid,
    restarts: int,
    stats: dict,
) -> str:
    """HTMX partial: live status + stats rows for maker page."""
    if running:
        badge = (
            '<span class="inline-flex items-center gap-1">'
            '<span class="w-2 h-2 rounded-full bg-emerald-400'
            ' animate-pulse"></span>'
            '<span class="text-emerald-400">running</span>'
            '</span>'
        )
        pid_txt = html.escape(str(pid or "?"))
    else:
        badge = (
            '<span class="inline-flex items-center gap-1">'
            '<span class="w-2 h-2 rounded-full bg-red-500">'
            '</span>'
            '<span class="text-red-400">stopped</span>'
            '</span>'
        )
        pid_txt = "--"

    orders_placed = stats.get("orders_placed", "--")
    active_orders = stats.get("active_orders", "--")
    mid_prices = stats.get("mid_prices", {})
    mid_txt = (
        html.escape(str(next(iter(mid_prices.values()), "--")))
        if mid_prices else "--"
    )
    errors = stats.get("errors", [])
    last_err = (
        f'<span class="text-red-400">'
        f'{html.escape(errors[-1])}</span>'
        if errors else
        '<span class="text-slate-600">none</span>'
    )
    spread_bps = stats.get("spread_bps", "--")

    rows = [
        ("Status", badge),
        ("PID", pid_txt),
        ("Restarts", str(restarts)),
        ("Orders placed", str(orders_placed)),
        ("Active orders", str(active_orders)),
        ("Mid price", mid_txt),
        ("Spread (bps)", str(spread_bps)),
        ("Last error", last_err),
    ]
    cells = "".join(
        f'<tr>'
        f'<td class="text-slate-500 pr-6 py-0.5 w-40">{k}</td>'
        f'<td class="font-mono">{v}</td>'
        f'</tr>'
        for k, v in rows
    )
    return f'<table class="text-xs">{cells}</table>'


def maker_page(
    running: bool,
    pid,
    restarts: int,
    cfg: dict,
) -> str:
    """Full Market Maker management page."""
    # Status section with live polling
    status_card = _card(
        "Status",
        '<div id="maker-live" '
        'hx-get="./x/maker-live" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600 text-xs">loading...</span>'
        '</div>',
    )

    # Controls
    controls_card = _card(
        "Controls",
        '''<div class="flex flex-wrap gap-2">
  <button class="bg-emerald-900/60 text-emerald-400
    px-3 py-1.5 rounded text-xs border border-emerald-800
    hover:bg-emerald-800 cursor-pointer"
    hx-post="./api/maker/start"
    hx-target="#maker-ctrl-result"
    hx-swap="innerHTML">Start</button>
  <button class="bg-red-900/40 text-red-400
    px-3 py-1.5 rounded text-xs border border-red-900
    hover:bg-red-900 cursor-pointer"
    hx-post="./api/maker/stop"
    hx-target="#maker-ctrl-result"
    hx-swap="innerHTML">Stop</button>
  <button class="bg-amber-900/40 text-amber-400
    px-3 py-1.5 rounded text-xs border border-amber-900
    hover:bg-amber-900 cursor-pointer"
    hx-post="./api/maker/restart"
    hx-target="#maker-ctrl-result"
    hx-swap="innerHTML">Restart</button>
  <span id="maker-ctrl-result" class="text-xs self-center">
  </span>
</div>''',
    )

    # Config form
    spread_bps = cfg.get("spread_bps", 20)
    qty = cfg.get("qty", 10)
    symbol_id = cfg.get("symbol_id", 10)
    refresh_ms = cfg.get("refresh_ms", 500)
    levels = cfg.get("levels", 5)

    def _field(label, name, value, hint=""):
        h = (
            f'<span class="text-slate-600 text-[11px]">'
            f'{html.escape(hint)}</span>'
            if hint else ""
        )
        return (
            f'<div>'
            f'<label class="block text-xs text-slate-400 mb-1">'
            f'{html.escape(label)}</label>'
            f'<input type="number" name="{name}" '
            f'value="{value}" '
            f'class="w-full bg-slate-950 border border-slate-700 '
            f'rounded px-2 py-1 text-xs text-slate-200 '
            f'focus:border-slate-500 focus:outline-none" />'
            f'{h}'
            f'</div>'
        )

    config_card = _card(
        "Config",
        f'''<form
  hx-post="./api/maker/config"
  hx-target="#maker-cfg-result"
  hx-swap="innerHTML">
  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3 mb-3">
    {_field("Spread (bps)", "spread_bps", spread_bps,
            "bid/ask spread")}
    {_field("Qty per level", "qty", qty,
            "order size multiplier")}
    {_field("Symbol ID", "symbol_id", symbol_id,
            "which symbol to make")}
    {_field("Refresh (ms)", "refresh_ms", refresh_ms,
            "quote refresh interval")}
    {_field("Levels", "levels", levels,
            "depth levels per side")}
  </div>
  <div class="flex items-center gap-3">
    <button type="submit"
      class="bg-blue-900/60 text-blue-400 px-4 py-1.5
        rounded text-xs border border-blue-800
        hover:bg-blue-800 cursor-pointer">
      Save &amp; Restart
    </button>
    <span id="maker-cfg-result" class="text-xs"></span>
  </div>
</form>''',
    )

    # Live stats card
    stats_card = _card(
        "Live Stats",
        '<div hx-get="./x/maker-status" '
        'hx-trigger="load, every 3s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600 text-xs">loading...</span>'
        '</div>',
    )

    content = f"""
{status_card}
{controls_card}
{config_card}
{stats_card}"""
    return layout("Market Maker", content, "./maker")
