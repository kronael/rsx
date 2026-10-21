#!/usr/bin/env bash
# Whole-exchange walkthrough → demo/exchange.gif.
#
# The ARCHITECTURE demo: how an order flows across RSX's separate processes
# (GW → Risk → ME → back), the two transports (casting/UDP live, WAL/TCP
# cold), and the scale-out axes (ME per symbol) — distinct from the dashboard
# tour (the operator toolset, rsx-playground/demo). Mixes browser shots of
# the dashboard's Topology/WAL pages with terminal recordings (order trace, rsx-cli WAL
# dump, the 3-book board) and caption cards (demo/cards/*.html).
#
#   ./demo/exchange-walkthrough.sh              # full: fresh trio + capture
#   SKIP_PREP=1 ./demo/exchange-walkthrough.sh  # cluster already up + fresh
#
# Prep boots a fresh trio; the capture records the traced order first.
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root

BASE=http://localhost:49171
OUT=tmp/exchange-walkthrough
GIF=demo/exchange.gif
PY=rsx-playground/.venv/bin/python3
CLI=target/debug/rsx-cli

# ── prep: fresh trio (PENGU/SOL/BTC + maker) ─────────────────────────
if [[ "${SKIP_PREP:-}" != 1 ]]; then
  echo "== fresh trio (reset + make demo — takes a few minutes)"
  ./rsx-playground/playground reset --yes
  make demo
fi
mkdir -p "$OUT"           # after reset — reset wipes tmp/
rm -f "$OUT"/*.png "$OUT"/*.cast "$OUT"/*.gif
curl -sf -o /dev/null "$BASE/overview" \
  || { echo "dashboard not up — make demo failed?" >&2; exit 1; }
[[ -x "$CLI" ]] || cargo build -q -p rsx-cli

echo "== waiting for a live quoted book"
for _ in $(seq 30); do
  asks=$(curl -sf "$BASE/api/book/10" | $PY -c \
    "import json,sys; b=json.load(sys.stdin); print(len(b.get('asks') or []))" \
    2>/dev/null || echo 0)
  [[ "$asks" -gt 0 ]] && break
  sleep 2
done
[[ "$asks" -gt 0 ]] || { echo "book never quoted" >&2; exit 1; }

rec() { # <name> <cols> <rows> <cmd...>
  local name=$1 cols=$2 rows=$3; shift 3
  TERM=xterm-256color asciinema rec --overwrite --quiet \
    --cols "$cols" --rows "$rows" -c "$*" "$OUT/$name.cast"
  agg --theme github-dark --font-size 16 --idle-time-limit 2 \
    --last-frame-duration 1 "$OUT/$name.cast" "$OUT/$name.gif" 2>/dev/null
  echo "  rec $name"
}

# ── terminal captures (order trace FIRST — freshest path) ────────────
rec trace 100 24 "$PY demo/trace_order.py"

rec walpeek 132 24 "bash demo/wal_peek.sh"

rec trio 96 18 "$PY demo/run.py"

# ── browser captures ─────────────────────────────────────────────────
shot() { agent-browser screenshot "$OUT/$1.png" >/dev/null; echo "  shot $1"; }
page() {
  agent-browser open "$BASE/$1" >/dev/null
  agent-browser wait --load networkidle >/dev/null 2>&1 || true
  sleep "$2"
}

agent-browser set viewport 1280 800 >/dev/null

# topology: the process graph, then a component detail + affinity map
page topology 4
agent-browser click "#topo-node-matching" >/dev/null 2>&1 || true
sleep 2;  shot 01-topology
agent-browser eval 'window.scrollTo(0, document.body.scrollHeight)' \
  >/dev/null 2>&1 || true
sleep 2;  shot 02-topology-detail

# the WAL page after the traced order: lifecycle events + tips
page wal 4;  shot 21-wal-page

# caption cards
card() { # <file-stem>
  agent-browser open "file://$PWD/demo/cards/$1.html" >/dev/null
  sleep 0.6
  shot "$1"
}
card 00-title
card 10-journey
card 20-tape
card 30-scale
card 40-end

# ── assemble ─────────────────────────────────────────────────────────
$PY demo/assemble_exchange_gif.py "$OUT" "$GIF"
gifsicle -O3 --lossy=60 -b "$GIF" 2>/dev/null || true
echo "  final: $(du -k "$GIF" | cut -f1) KB"
echo "== done — cluster left running (make stop to stop)"
