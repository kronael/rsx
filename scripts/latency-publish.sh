#!/usr/bin/env bash
#
# scripts/latency-publish.sh — drive the F1 latency probe
# under a fixed N-orders load and write measured p50/p99 to
# bench-baseline.json so the README "What's measured" table
# can stop calling the GW->ME->GW number a "design budget."
#
# Pre-conditions:
#   - rsx-playground/playground start-all is up (gateway,
#     risk, ME, marketdata, mark, recorder all healthy)
#   - rsx-maker is running (so probe orders match against
#     resting liquidity)
#   - The local test JWT secret is set in env (the playground
#     mints one on `start-all`)
#
# Usage:
#   make latency-publish               # default N=2000
#   N=10000 make latency-publish       # heavier load
#
# Output:
#   - prints p50/p99/count to stdout
#   - merges {"e2e_us": {"p50": ..., "p99": ..., "n": ...,
#     "ts": ...}} into bench-baseline.json (creates if absent)
#
# This script is the founder-runnable companion to commit
# `bded133` (F1 probe shipped) and closes the audit's T2.3
# "the <50us claim is unmeasured" gap. Once `bench-baseline.json`
# carries an `e2e_us` block, README §"What's measured" can
# move that line out of the "design budget" column.

set -euo pipefail

N=${N:-2000}
ENDPOINT=${ENDPOINT:-http://127.0.0.1:49171}
SYMBOL=${SYMBOL:-10}
BASELINE=${BASELINE:-bench-baseline.json}
WARMUP=${WARMUP:-50}

cd "$(dirname "$0")/.."

echo "[latency-publish] endpoint=${ENDPOINT} symbol=${SYMBOL} N=${N} warmup=${WARMUP}"

# Health check first — fail fast if the cluster isn't up.
if ! curl -fsS -m 2 "${ENDPOINT}/healthz" >/dev/null 2>&1; then
    echo "[latency-publish] cluster not reachable at ${ENDPOINT}" >&2
    echo "[latency-publish] start it with: ./rsx-playground/playground start-all" >&2
    exit 2
fi

# Drain any stale latencies from the in-memory ring so the
# percentiles we read back reflect this run only.
curl -fsS -X POST "${ENDPOINT}/api/latency/reset" >/dev/null 2>&1 \
    || echo "[latency-publish] /api/latency/reset not present, continuing"

# Warmup: discard the first WARMUP probes (cold caches, slow path).
echo "[latency-publish] warmup ${WARMUP} probes..."
for _ in $(seq 1 "$WARMUP"); do
    curl -fsS -X POST "${ENDPOINT}/api/latency-probe?symbol_id=${SYMBOL}" \
        >/dev/null 2>&1 || true
done
curl -fsS -X POST "${ENDPOINT}/api/latency/reset" >/dev/null 2>&1 || true

# Measurement run.
echo "[latency-publish] measurement ${N} probes..."
ok=0
fail=0
for i in $(seq 1 "$N"); do
    if curl -fsS -X POST "${ENDPOINT}/api/latency-probe?symbol_id=${SYMBOL}" \
        >/dev/null 2>&1; then
        ok=$((ok + 1))
    else
        fail=$((fail + 1))
    fi
done
echo "[latency-publish] probe results: ok=${ok} fail=${fail}"

if [ "$ok" -eq 0 ]; then
    echo "[latency-publish] all probes failed; aborting" >&2
    exit 3
fi

# Read percentiles from the server's own e2e_latencies ring.
RAW=$(curl -fsS "${ENDPOINT}/api/latency")
P50=$(echo "$RAW" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('e2e',{}).get('p50','null'))")
P99=$(echo "$RAW" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('e2e',{}).get('p99','null'))")
COUNT=$(echo "$RAW" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('e2e',{}).get('count',0))")

echo "[latency-publish] e2e p50=${P50}us p99=${P99}us count=${COUNT}"

# Merge into bench-baseline.json without clobbering other keys.
python3 - "$BASELINE" "$P50" "$P99" "$COUNT" <<'PYEOF'
import json, sys, os, time
path, p50, p99, count = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
data = {}
if os.path.exists(path):
    with open(path) as f:
        try:
            data = json.load(f)
        except json.JSONDecodeError:
            data = {}
data["e2e_us"] = {
    "p50": float(p50) if p50 != "null" else None,
    "p99": float(p99) if p99 != "null" else None,
    "n": int(count),
    "ts": int(time.time()),
}
with open(path, "w") as f:
    json.dump(data, f, indent=2, sort_keys=True)
    f.write("\n")
print(f"[latency-publish] merged into {path}")
PYEOF
