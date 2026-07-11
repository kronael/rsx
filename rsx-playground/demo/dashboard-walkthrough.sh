#!/usr/bin/env bash
# Comprehensive dashboard walkthrough → rsx-playground/demo/dashboard.gif.
# Drives a real browser (agent-browser) over the live RSX playground dashboard,
# screenshots the key pages, and assembles them into a looping GIF. The system
# must be up first: `make demo` (3 tokens trading live).
#
#   ./rsx-playground/demo/dashboard-walkthrough.sh   # from the repo root
set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root

BASE=http://localhost:49171
OUT=tmp/walkthrough
GIF=rsx-playground/demo/dashboard.gif
mkdir -p "$OUT"

curl -sf -o /dev/null "$BASE/overview" \
  || { echo "dashboard not up — run 'make demo' first" >&2; exit 1; }

# (page-path, output-name) — the teaching arc: boot → topology → live book →
# orders → maker → latency → cast → embedded terminal → invariant checks.
pages=(
  "overview 01-overview" "topology 02-topology" "book 03-book"
  "orders 04-orders" "maker 05-maker" "latency 06-latency"
  "cast 07-cast" "terminal 08-terminal" "verify 09-verify"
)
for entry in "${pages[@]}"; do
  set -- $entry
  agent-browser open "$BASE/$1" >/dev/null 2>&1
  agent-browser wait --load networkidle >/dev/null 2>&1
  sleep 1
  agent-browser screenshot "$OUT/$2.png" >/dev/null 2>&1
  echo "  captured $2"
done

python3 - "$OUT" "$GIF" <<'PY'
import glob, os, sys
from PIL import Image
out, gif = sys.argv[1], sys.argv[2]
files = sorted(glob.glob(f"{out}/[0-9]*.png"))
frames = [Image.open(f).convert("RGB").resize((960, 433)) for f in files]
pf = [im.convert("P", palette=Image.ADAPTIVE, colors=128) for im in frames]
pf[0].save(gif, save_all=True, append_images=pf[1:],
           duration=2200, loop=0, optimize=True)
print(f"  {gif}: {len(files)} pages, {os.path.getsize(gif)} bytes")
PY
