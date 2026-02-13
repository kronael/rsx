"""Inline HTML generation for RSX Playground dashboard.

Uses Tailwind Play CDN (script tag, JIT compiler in browser)
+ HTMX for interactivity. All URLs relative for proxy compat.
"""

TABS = [
    ("Overview", "./overview"),
    ("Topology", "./topology"),
    ("Book", "./book"),
    ("Risk", "./risk"),
    ("WAL", "./wal"),
    ("Logs", "./logs"),
    ("Control", "./control"),
    ("Faults", "./faults"),
    ("Verify", "./verify"),
    ("Orders", "./orders"),
    ("Docs", "http://localhost:8001", True),
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
<script src="https://unpkg.com/htmx.org@2.0.4"></script>
<script src="https://unpkg.com/htmx-ext-sse@2.2.2/sse.js">
</script>
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
            '<select id="scenario" class="bg-slate-950 '
            'border border-slate-700 text-slate-300 '
            'px-2 py-1 rounded text-xs">'
            '<option value="minimal">minimal (1Z)</option>'
            '<option value="duo">duo (2Z)</option>'
            '<option value="full">full (3)</option>'
            '<option value="stress">stress (M3S)</option>'
            '</select>'
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
    content = f"""
{health}
{procs}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{metrics}
{rings}
</div>
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{wal_status}
{logs_tail}
</div>
{invariants}"""
    return layout("Overview", content, "./overview")


# ── Screen 2: Topology ──────────────────────────────────

def topology_page():
    graph = _card(
        "Process Graph",
        """<pre class="text-slate-400 text-xs leading-relaxed
  whitespace-pre">
  [Gateway] ---CMP/UDP---> [Risk] ---CMP/UDP---> [ME-BTCUSD]
      |                       |                         |
      +-------UDP/CMP---------+                         |
                                                        v
                                               [Marketdata]
                                                        |
  [Recorder] <-----------------WAL/TCP-----------------+

  [Mark] <-------CMP/UDP------- [ME]
</pre>""",
    )
    affinity = _card(
        "Core Affinity Map",
        '<div hx-get="./x/core-affinity" '
        'hx-trigger="load, every 5s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    cmp = _card(
        "CMP Connections",
        '<div hx-get="./x/cmp-flows" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    procs = _card(
        "Process List",
        '<div hx-get="./x/processes" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    content = f"""
{graph}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{affinity}
{cmp}
</div>
{procs}"""
    return layout("Topology", content, "./topology")


# ── Screen 3: Book ───────────────────────────────────────

def book_page():
    selector = """
<div class="flex flex-wrap items-center gap-3 mb-3">
  <select id="book-symbol"
    onchange="htmx.trigger('#book-data','load')"
    class="w-full sm:w-auto bg-slate-950 border border-slate-700
      text-slate-300 px-2 py-1 rounded text-xs">
    <option value="10">PENGU</option>
    <option value="3">SOL</option>
    <option value="1">BTC</option>
    <option value="2">ETH</option>
  </select>
</div>"""
    ladder = _card(
        "Orderbook Ladder",
        f"""{selector}
<div id="book-data" hx-get="./x/book"
     hx-trigger="load, every 1s" hx-swap="innerHTML"
     hx-vals="js:{{symbol_id:
       document.getElementById('book-symbol').value}}">
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
    heatmap = _card(
        "Position Heatmap (users x symbols)",
        '<div hx-get="./x/position-heatmap" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    margin = _card(
        "Margin Ladder",
        '<div hx-get="./x/margin-ladder" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    funding = _card(
        "Funding",
        '<div hx-get="./x/funding" '
        'hx-trigger="load, every 2s" '
        'hx-swap="innerHTML">'
        '<span class="text-slate-600">loading...</span>'
        '</div>',
    )
    liq = _card(
        "Liquidation Queue",
        '<div hx-get="./x/liquidations" '
        'hx-trigger="load, every 2s" hx-swap="innerHTML">'
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
  <input type="number" id="risk-uid" value="1"
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
    hx-post="./api/users" hx-swap="none">Create User</button>
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
{heatmap}
{margin}
<div class="grid grid-cols-1 md:grid-cols-2 gap-3">
{funding}
{liq}
</div>
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
        """<div class="flex flex-wrap items-center gap-2 mb-3">
  <select id="wal-filter"
    class="w-full sm:w-auto bg-slate-950 border border-slate-700
      text-slate-300 px-2 py-1 rounded text-xs">
    <option value="">all</option>
    <option value="ORDER_ACCEPTED">ORDER_ACCEPTED</option>
    <option value="FILL">FILL</option>
    <option value="MARGIN_CHECK">MARGIN_CHECK</option>
  </select>
</div>
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
      class="flex-1 min-w-[200px] bg-slate-950 border
        border-slate-700 text-slate-300 px-2 py-1 rounded text-xs"
      placeholder="Smart search: 'gateway error order' or just search text (press / to focus, Ctrl+L to clear)"
      onkeydown="handleSmartSearch(event)">
    <button class="bg-slate-800 text-slate-400 px-3 py-1 rounded
      text-xs border border-slate-700 hover:bg-slate-700"
      onclick="clearAllFilters()">Clear All</button>
  </div>
  <div class="hidden" id="filter-dropdowns">
    <select id="log-process"
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
    <select id="log-level"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="">all levels</option>
      <option value="error">error</option>
      <option value="warn">warn</option>
      <option value="info">info</option>
      <option value="debug">debug</option>
    </select>
    <input type="text" id="log-search"
      class="w-full sm:w-44 bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs"
      placeholder="search...">
  </div>
</div>
<div id="log-view" class="max-h-[500px] overflow-y-auto
  overflow-x-auto"
     hx-get="./x/logs" hx-trigger="load, every 2s"
     hx-swap="innerHTML"
     hx-include="#log-process, #log-level, #log-search">
  <span class="text-slate-600">loading...</span>
</div>
<div id="log-modal" class="hidden fixed inset-0 bg-black/80
  flex items-center justify-center z-50" onclick="closeModal()">
  <div class="bg-slate-900 border border-slate-700 rounded-lg
    p-4 max-w-4xl max-h-[80vh] overflow-auto m-4"
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
    return layout(
        "Control",
        scenario_selector + grid + notes + resources,
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
    form = """
<form hx-post="./api/orders/test" hx-target="#order-result"
      hx-swap="innerHTML"
      class="flex flex-wrap gap-3 items-end mb-3">
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Symbol</label>
    <select name="symbol_id"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="10">PENGU</option>
      <option value="3">SOL</option>
      <option value="1">BTC</option>
      <option value="2">ETH</option>
    </select>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Side</label>
    <select name="side"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="buy">BUY</option>
      <option value="sell">SELL</option>
    </select>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Type</label>
    <select name="order_type"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="limit">LIMIT</option>
      <option value="market">MARKET</option>
      <option value="post_only">POST_ONLY</option>
    </select>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Price</label>
    <input type="text" name="price" value="50000"
      class="w-full sm:w-24 bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">Qty</label>
    <input type="text" name="qty" value="1.0"
      class="w-full sm:w-20 bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">TIF</label>
    <select name="tif"
      class="w-full sm:w-auto bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
      <option value="GTC">GTC</option>
      <option value="IOC">IOC</option>
      <option value="FOK">FOK</option>
    </select>
  </div>
  <div class="w-full sm:w-auto">
    <label class="text-[10px] text-slate-500 uppercase
      tracking-wider block mb-1">User ID</label>
    <input type="number" name="user_id" value="1"
      class="w-full sm:w-20 bg-slate-950 border border-slate-700
        text-slate-300 px-2 py-1 rounded text-xs">
  </div>
  <div class="flex items-end gap-3">
    <label class="text-[10px] text-slate-500 flex items-center
      gap-1 cursor-pointer">
      <input type="checkbox" name="reduce_only"
        class="accent-blue-500"> RO</label>
    <label class="text-[10px] text-slate-500 flex items-center
      gap-1 cursor-pointer">
      <input type="checkbox" name="post_only"
        class="accent-blue-500"> PO</label>
  </div>
  <button type="submit"
    class="w-full sm:w-auto bg-emerald-900/60 text-emerald-400
      px-3 py-1 rounded text-xs border border-emerald-800
      hover:bg-emerald-800 cursor-pointer">Submit</button>
</form>
<div class="flex flex-wrap gap-2 mb-3">
  <button class="bg-slate-800 text-slate-400
    px-3 py-1 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/batch"
    hx-target="#order-result"
    hx-swap="innerHTML">Batch (10)</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/random"
    hx-target="#order-result"
    hx-swap="innerHTML">Random (5)</button>
  <button class="bg-amber-900/40 text-amber-400
    px-3 py-1 rounded text-xs border border-amber-900
    hover:bg-amber-900 cursor-pointer"
    hx-post="./api/orders/stress"
    hx-target="#order-result"
    hx-swap="innerHTML">Stress (100)</button>
  <button class="bg-slate-800 text-slate-400
    px-3 py-1 rounded text-xs border border-slate-700
    hover:bg-slate-700 cursor-pointer"
    hx-post="./api/orders/invalid"
    hx-target="#order-result"
    hx-swap="innerHTML">Invalid</button>
</div>
<div id="order-result"></div>"""
    lifecycle = _card(
        "Order Lifecycle Trace",
        """<div class="flex flex-wrap items-center gap-2 mb-3">
  <input type="text" id="trace-oid" placeholder="oid..."
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


def _bar(pct, color="emerald"):
    """Progress bar for backpressure / resource usage."""
    if pct > 80:
        color = "red"
    elif pct > 50:
        color = "amber"
    w = max(1, min(100, pct))
    return (
        f'<div class="flex items-center gap-2">'
        f'<div class="flex-1 bg-slate-800 rounded h-2">'
        f'<div class="bg-{color}-500 h-2 rounded" '
        f'style="width:{w}%"></div></div>'
        f'<span class="text-[10px] text-slate-500 w-8 '
        f'text-right">{pct}%</span></div>'
    )


def _metric(label, value, color="slate-300"):
    """Single metric in a strip."""
    return (
        f'<div class="text-center">'
        f'<div class="text-[10px] text-slate-500 '
        f'uppercase tracking-wider">{label}</div>'
        f'<div class="text-sm text-{color} '
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
    if score >= 80:
        color = "emerald"
    elif score >= 50:
        color = "amber"
    else:
        color = "red"
    bar_w = max(1, score)
    return (
        f'<div class="flex items-center gap-4">'
        f'<div class="flex-1 bg-slate-800 rounded h-4">'
        f'<div class="bg-{color}-500 h-4 rounded flex '
        f'items-center justify-center text-[10px] '
        f'font-bold text-white" '
        f'style="width:{bar_w}%">{score}</div></div>'
        f'<span class="text-xs font-semibold '
        f'text-{color}-400 uppercase">'
        f'{"green" if score >= 80 else "yellow" if score >= 50 else "red"}'
        f'</span></div>'
    )


def render_key_metrics(procs, wal_streams):
    running = sum(
        1 for p in procs if p.get("state") == "running")
    wal_files = sum(s.get("files", 0) for s in wal_streams)
    return (
        '<div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-6 '
        'gap-4">'
        + _metric("Processes", f"{running}/{len(procs)}",
                  "emerald-400")
        + _metric("Active Orders", "--", "slate-500")
        + _metric("Positions", "--", "slate-500")
        + _metric("Msgs/sec", "--", "slate-500")
        + _metric("WAL Files", str(wal_files), "blue-400")
        + _metric("Errors", "0", "emerald-400")
        + '</div>'
    )


def render_ring_pressure():
    """Static placeholder until CMP stats are available."""
    rings = [
        ("GW -> Risk", 0),
        ("Risk -> ME", 0),
        ("ME -> Mktdata", 0),
        ("ME -> Recorder", 0),
    ]
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
    return _table(
        ["", "Name", "PID", "CPU%", "Mem",
         "Uptime", "State", "Actions"],
        rows,
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


def render_cmp_flows():
    """Static placeholder until CMP stats available."""
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
    return _table(
        ["", "Name", "State", "PID", "Uptime", "Actions"],
        rows,
    )


def render_resource_usage(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found</span>')
    html = '<div class="space-y-3">'
    for p in processes:
        if p.get("state") != "running":
            continue
        cpu_str = p.get("cpu", "0%").rstrip("%")
        try:
            cpu = float(cpu_str)
        except ValueError:
            cpu = 0
        mem = p.get("mem", "-")
        html += (
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
    html += '</div>'
    return html


# ── Faults grid ──────────────────────────────────────────

def render_faults_grid(processes):
    if not processes:
        return ('<span class="text-slate-600">'
                'no processes found</span>')
    rows = ""
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
    return _table(
        ["", "Name", "State", "PID", "Actions"], rows,
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
            f'<td {_TD}>{s["name"]}</td>'
            f'<td {_TD}>{s["files"]}</td>'
            f'<td {_TD}>{s["total_size"]}</td></tr>'
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
            f'<td {_TD}>{s["name"]}</td>'
            f'<td {_TD}>{s["files"]}</td>'
            f'<td {_TD}>{s["total_size"]}</td>'
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
            f'<td {_TD}>{f["stream"]}</td>'
            f'<td {_TD}>{f["name"]}</td>'
            f'<td {_TD}>{f["size"]}</td>'
            f'<td {_TD}>{f["modified"]}</td></tr>'
        )
    return _table(
        ["Stream", "File", "Size", "Modified"], rows,
    )


def render_wal_lag():
    """Placeholder until live lag data available."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes to see lag data</span>')


def render_wal_rotation():
    """Placeholder until live rotation data available."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes to see rotation status'
            '</span>')


def render_wal_timeline():
    """Placeholder until live event stream available."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes to see timeline</span>')


# ── Logs ─────────────────────────────────────────────────

def render_logs(lines):
    if not lines:
        return ('<span class="text-slate-600">'
                'no log lines</span>')
    html = ""
    for i, line in enumerate(lines):
        cls = "text-slate-400"
        low = line.lower()
        if " error " in low:
            cls = "text-red-400"
        elif " warn " in low:
            cls = "text-amber-400"
        elif " debug " in low:
            cls = "text-slate-600"
        escaped_line = line.replace('"', '&quot;').replace("'", "&#39;")
        html += (
            f'<div class="{cls} text-xs py-0.5 font-mono '
            f'whitespace-pre-wrap break-all cursor-pointer '
            f'hover:bg-slate-800 px-1 rounded group relative" '
            f'onclick="showFullLine(this, {i})">'
            f'<span class="group-hover:opacity-100 opacity-0 '
            f'absolute right-1 top-1 text-[10px] text-slate-500">'
            f'click to expand</span>'
            f'{line}</div>\n'
        )
    return html


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
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD} class="text-red-400 text-xs '
            f'max-w-xs truncate">{pattern}</td>'
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
        if c["status"] == "pass":
            badge = ("bg-emerald-950 text-emerald-400 "
                     "border border-emerald-900")
            label = "PASS"
        elif c["status"] == "fail":
            badge = ("bg-red-950 text-red-400 "
                     "border border-red-900")
            label = "FAIL"
        else:
            badge = ("bg-amber-950 text-amber-400 "
                     "border border-amber-900")
            label = "SKIP"
        detail = ""
        if c.get("detail"):
            detail = (
                f'<div class="text-[10px] text-slate-500 '
                f'mt-0.5">{c["detail"]}</div>'
            )
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}><span class="{badge} px-2 py-0.5 '
            f'rounded text-[10px] font-semibold">'
            f'{label}</span></td>'
            f'<td {_TD}>{c["name"]}{detail}</td>'
            f'<td {_TD} class="text-slate-500 text-[10px]">'
            f'{c.get("time", "-")}</td></tr>'
        )
    return _table(["Status", "Check", "Last Run"], rows)


def render_reconciliation():
    """Placeholder reconciliation checks."""
    items = [
        ("Frozen margin vs computed", "skip",
         "requires live system"),
        ("Shadow book vs ME book", "skip",
         "requires live system"),
        ("Mark price vs index", "skip",
         "requires live system"),
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


def render_latency_regression():
    """Placeholder latency regression."""
    return (
        '<div class="space-y-2">'
        '<div class="flex items-center gap-3">'
        '<span class="text-xs w-40">GW->ME->GW p99</span>'
        '<span class="text-slate-500 text-xs">--</span>'
        '<span class="text-[10px] text-slate-600">'
        '(baseline 50us)</span></div>'
        '<div class="flex items-center gap-3">'
        '<span class="text-xs w-40">ME match p99</span>'
        '<span class="text-slate-500 text-xs">--</span>'
        '<span class="text-[10px] text-slate-600">'
        '(baseline 500ns)</span></div>'
        '</div>'
    )


# ── Orders ───────────────────────────────────────────────

SYMBOL_NAMES = {
    "1": "BTC", "2": "ETH", "3": "SOL", "10": "PENGU",
}


def render_recent_orders(orders):
    if not orders:
        return ('<span class="text-slate-600">'
                'no orders yet</span>')
    rows = ""
    for o in orders:
        sym = o.get("symbol", "-")
        sym_name = SYMBOL_NAMES.get(sym, sym)
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
        rows += (
            f'<tr class="hover:bg-slate-800/50">'
            f'<td {_TD}>{o.get("cid", "-")}</td>'
            f'<td {_TD}>{sym_name}</td>'
            f'<td {_TD}>{o.get("side", "-")}</td>'
            f'<td {_TD}>{o.get("price", "-")}</td>'
            f'<td {_TD}>{o.get("qty", "-")}</td>'
            f'<td {_TD}>{tif}</td>'
            f'<td {_TD}>{flags.strip()}</td>'
            f'<td {_TD}>{o.get("status", "-")}</td>'
            f'<td {_TD}>{o.get("ts", "-")}</td>'
            f'<td {_TD}>{cancel}</td></tr>'
        )
    return _table(
        ["CID", "Symbol", "Side", "Price", "Qty",
         "TIF", "Flags", "Status", "Time", ""],
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
            f'<td {_TD} class="text-slate-500">{key}</td>'
            f'<td {_TD}>{val}</td></tr>'
        )
    return _table(["Field", "Value"], rows)


def render_position_heatmap():
    """Placeholder position heatmap."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes and create users to see '
            'position data</span>')


def render_margin_ladder():
    """Placeholder margin ladder."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes to see margin data</span>')


def render_funding():
    """Placeholder funding tracking."""
    return ('<span class="text-slate-500 text-xs">'
            'start RSX processes to see funding data</span>')


def render_risk_latency():
    """Placeholder risk check latency."""
    return (
        '<div class="flex gap-6">'
        + _metric("p50", "--", "slate-500")
        + _metric("p95", "--", "slate-500")
        + _metric("p99", "--", "slate-500")
        + _metric("max", "--", "slate-500")
        + '</div>'
    )
