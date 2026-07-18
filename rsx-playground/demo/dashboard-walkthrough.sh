#!/usr/bin/env bash
# Interactive dashboard walkthrough → rsx-playground/demo/dashboard.gif.
#
# Records the dashboard as an OPERATOR'S TOOLSET against the live demo scene
# (demo_scenario.py: calm → +45% PENGU pump with a real risk-engine
# liquidation → retrace), so every page shows real action: an order landing,
# the book marching up, a crash healing, the invariants holding after the
# liquidation. Drives a real browser via agent-browser, screenshots each
# step, PIL-stitches + gifsicle-shrinks into a looping GIF.
#
#   ./rsx-playground/demo/dashboard-walkthrough.sh          # full: prep + run + capture
#   SKIP_PREP=1 ./rsx-playground/demo/dashboard-walkthrough.sh  # scene cluster already prepped
#
# prep boots a FRESH PENGU-only cluster (several minutes: reset + build +
# stage the victim short). Capture starts the timed scene in the background
# and shoots promptly — the demo cluster's order path degrades minutes after
# boot (BUGS.md RISK-CMP-DESYNC-ORDER-EVENT-STARVE), so don't dawdle between
# prep and capture.
set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

BASE=http://localhost:49171
OUT=tmp/walkthrough
GIF=rsx-playground/demo/dashboard.gif
SCENE_LOG=$OUT/scene.log

# ── scene ────────────────────────────────────────────────────────────
if [ "${SKIP_PREP:-}" != 1 ]; then
  echo "== demo-scene prep (fresh cluster — takes a few minutes)"
  ./rsx-playground/playground demo-scene prep
fi
# after prep — its `reset` wipes tmp/, so only now is $OUT safe to make
mkdir -p "$OUT"
rm -f "$OUT"/[0-9]*.png "$SCENE_LOG"
curl -sf -o /dev/null "$BASE/overview" \
  || { echo "dashboard not up — demo-scene prep failed?" >&2; exit 1; }

echo "== demo-scene run (background) → $SCENE_LOG"
./rsx-playground/playground demo-scene run >"$SCENE_LOG" 2>&1 &
SCENE_PID=$!
cleanup() {
  kill "$SCENE_PID" 2>/dev/null || true
  pkill -f "demo_scenario.py run" 2>/dev/null || true
  ./rsx-playground/playground demo-scene clean >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Block until a marker line appears in the scene log (or timeout — the
# capture goes on; a missed marker just means less drama in that frame).
wait_scene() { # <marker> <timeout_s>
  local deadline=$(( $(date +%s) + $2 ))
  until grep -q "$1" "$SCENE_LOG" 2>/dev/null; do
    [ "$(date +%s)" -ge "$deadline" ] && {
      echo "  (scene marker '$1' not seen in ${2}s — continuing)"; return 1; }
    sleep 1
  done
  echo "  scene: $1"
}

wait_scene "run: mid0" 30 || { echo "scene run failed" >&2; cat "$SCENE_LOG" >&2; exit 1; }

# ── capture ──────────────────────────────────────────────────────────
shot() { # <name>  (screenshot the current page)
  agent-browser screenshot "$OUT/$1.png" >/dev/null
  echo "  shot $1"
}
page() { # <path> <settle_s>  (open a page, let htmx cards fill)
  agent-browser open "$BASE/$1" >/dev/null
  agent-browser wait --load networkidle >/dev/null 2>&1 || true
  sleep "$2"
}

agent-browser set viewport 1280 800 >/dev/null

# 1-2 · calm: the managed cluster + its control surface
page overview 3;  shot 01-overview
page control 2;   shot 02-control

# 3-4 · order submission: click a quick BUY, show it landing
page orders 2;    shot 03-orders
agent-browser eval '
  [...document.querySelectorAll("button")].find(b =>
    (b.getAttribute("hx-vals")||"").includes("\"side\":\"buy\",\"qty\":\"20\"")
  ).click()' >/dev/null
sleep 3;          shot 04-orders-submit

# 5-7 · the book DURING the pump: three beats of the mid marching up
wait_scene "PUMP begins" 30 || true
page book 3;      shot 05-book-pump1
sleep 6;          shot 06-book-pump2
sleep 6;          shot 07-book-pump3

# 8 · the risk engine's liquidation just landed — positions page
wait_scene "LIQUIDATION confirmed" 60 || true
page risk 4;      shot 08-risk-liquidation

# 9-10 · fault injection: SIGKILL the recorder, watch the watcher heal it
page recovery 3
agent-browser eval '
  window.confirm = () => true;
  [...document.querySelectorAll("button")].find(b =>
    b.textContent.trim() === "Crash" &&
    b.parentElement.textContent.includes("recorder")
  ).click()' >/dev/null
sleep 0.8;        shot 09-recovery-inject   # instant: catch "recorder down"
sleep 8;          shot 10-recovery-healed   # watcher restarted it, new pid

# 11-12 · the tape: WAL streams + casting/replication flows
page wal 3;       shot 11-wal
page cast 3;      shot 12-cast

# 13 · latency, with its honest caption (playground↔gateway path, not the
# exchange's µs internals — benches own those); fire one real e2e probe
page latency 3
agent-browser eval '
  [...document.querySelectorAll("button")].find(b =>
    b.textContent.includes("Run one probe")).click()' >/dev/null
sleep 3;          shot 13-latency

# 14 · verify AFTER the liquidation: 10 invariants + reconciliation held
page verify 6;    shot 14-verify

kill "$SCENE_PID" 2>/dev/null || true
pkill -f "demo_scenario.py run" 2>/dev/null || true

# ── assemble ─────────────────────────────────────────────────────────
python3 - "$OUT" "$GIF" <<'PY'
import glob, os, sys
from PIL import Image
out, gif = sys.argv[1], sys.argv[2]
files = sorted(glob.glob(f"{out}/[0-9]*.png"))
frames = [Image.open(f).convert("RGB").resize((960, 600)) for f in files]
pal = [im.convert("P", palette=Image.ADAPTIVE, colors=128) for im in frames]
# book-pump beats run short so the mid visibly marches; the rest hold
durs = [1300 if "book-pump" in f else 2400 for f in files]
pal[0].save(gif, save_all=True, append_images=pal[1:],
            duration=durs, loop=0, optimize=True)
print(f"  {gif}: {len(files)} frames, {os.path.getsize(gif)//1024} KB")
PY
gifsicle -O3 --lossy=60 -b "$GIF" 2>/dev/null || true
echo "  final: $(du -k "$GIF" | cut -f1) KB"
echo "== done — cluster left running (./rsx-playground/playground stop-all to stop)"
