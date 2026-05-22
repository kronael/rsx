#!/usr/bin/env bash
#
# scripts/bench-gate-e2e.sh — E2E latency regression gate.
#
# Runs `latency-publish.sh` against a live cluster, reads
# the resulting `e2e_us.p50` from `bench-baseline.json`, and
# compares against a sealed reference in
# `bench-reference.json`. Fails if p50 regresses more than
# THRESHOLD percent (default 10).
#
# Unlike the Criterion-based `bench-gate.sh` which measures
# isolated unit-bench operations, this gate measures the
# full GW→ME→GW round-trip in a deployed cluster. It is
# meant for the CI lane that already brings up the
# playground / start-all pipeline.
#
# Why a separate "reference" file? `bench-baseline.json`
# is rolling — `make latency-publish` rewrites the
# `e2e_us` block on every run. The sealed reference here
# only changes when the founder explicitly accepts a new
# floor (e.g. after a deliberate perf push). This avoids
# the silent baseline-creep that would otherwise hide
# regressions.
#
# Usage:
#   bash scripts/bench-gate-e2e.sh                # N=200
#   N=2000 bash scripts/bench-gate-e2e.sh         # heavier
#   bash scripts/bench-gate-e2e.sh --save-reference  # snapshot
#
# CI typically:
#   1) ./rsx-playground/playground start-all
#   2) make bench-gate-e2e
#   3) ./rsx-playground/playground stop-all
#
# Exit codes:
#   0 — pass (p50 within THRESHOLD% of reference)
#   1 — fail (p50 regressed beyond threshold)
#   2 — environment problem (no cluster, missing files)

set -euo pipefail

THRESHOLD=${THRESHOLD:-10}
N=${N:-200}
REFERENCE=${REFERENCE:-bench-reference.json}
BASELINE=${BASELINE:-bench-baseline.json}
SAVE=0

for arg in "$@"; do
    case "$arg" in
        --save-reference) SAVE=1 ;;
        *) echo "unknown arg: $arg" >&2; exit 1 ;;
    esac
done

cd "$(dirname "$0")/.."

echo "[bench-gate-e2e] reference=${REFERENCE} baseline=${BASELINE} threshold=${THRESHOLD}% N=${N}"

# Drive the probe.
N="$N" bash scripts/latency-publish.sh

if [ ! -f "$BASELINE" ]; then
    echo "[bench-gate-e2e] baseline file missing: $BASELINE" >&2
    exit 2
fi

CUR_P50=$(python3 -c "
import json, sys
with open('$BASELINE') as f:
    d = json.load(f)
e2e = d.get('e2e_us') or {}
p = e2e.get('p50')
print(p if p is not None else 'null')
")
CUR_P99=$(python3 -c "
import json
with open('$BASELINE') as f:
    d = json.load(f)
e2e = d.get('e2e_us') or {}
p = e2e.get('p99')
print(p if p is not None else 'null')
")
CUR_N=$(python3 -c "
import json
with open('$BASELINE') as f:
    d = json.load(f)
e2e = d.get('e2e_us') or {}
print(e2e.get('n', 0))
")

if [ "$CUR_P50" = "null" ]; then
    echo "[bench-gate-e2e] no e2e_us.p50 in $BASELINE; probe likely failed" >&2
    exit 2
fi

echo "[bench-gate-e2e] current: p50=${CUR_P50}us p99=${CUR_P99}us n=${CUR_N}"

if [ "$SAVE" -eq 1 ]; then
    python3 - <<PYEOF
import json, time, os
src = json.load(open("$BASELINE"))
e2e = src.get("e2e_us") or {}
out = {
    "e2e_us": {
        "p50": e2e.get("p50"),
        "p99": e2e.get("p99"),
        "n": e2e.get("n"),
        "ts": int(time.time()),
    },
    "_comment": (
        "Sealed E2E latency reference. Updated only "
        "with --save-reference. The bench-gate-e2e "
        "script fails CI if e2e_us.p50 regresses "
        ">${THRESHOLD}% from this value."
    ),
}
with open("$REFERENCE", "w") as f:
    json.dump(out, f, indent=2, sort_keys=True)
    f.write("\n")
print(f"[bench-gate-e2e] saved reference to $REFERENCE")
PYEOF
    exit 0
fi

if [ ! -f "$REFERENCE" ]; then
    echo "[bench-gate-e2e] no reference file at $REFERENCE"
    echo "[bench-gate-e2e] create one with: $0 --save-reference"
    echo "[bench-gate-e2e] then commit it so CI has a stable target"
    # Exit 0 so a fresh repo doesn't fail the gate.
    exit 0
fi

REF_P50=$(python3 -c "
import json
with open('$REFERENCE') as f:
    d = json.load(f)
e2e = d.get('e2e_us') or {}
p = e2e.get('p50')
print(p if p is not None else 'null')
")
REF_P99=$(python3 -c "
import json
with open('$REFERENCE') as f:
    d = json.load(f)
e2e = d.get('e2e_us') or {}
p = e2e.get('p99')
print(p if p is not None else 'null')
")

if [ "$REF_P50" = "null" ]; then
    echo "[bench-gate-e2e] reference $REFERENCE has no e2e_us.p50" >&2
    exit 2
fi

echo "[bench-gate-e2e] reference: p50=${REF_P50}us p99=${REF_P99}us"

# Compute ratio.
RATIO=$(python3 -c "print(${CUR_P50} / ${REF_P50})")
PCT=$(python3 -c "print(round((${CUR_P50} / ${REF_P50} - 1) * 100, 2))")
LIMIT=$(python3 -c "print(1 + ${THRESHOLD} / 100)")
FAIL=$(python3 -c "print(1 if ${RATIO} > ${LIMIT} else 0)")

echo "[bench-gate-e2e] ratio=${RATIO} (${PCT}% vs ref)"

if [ "$FAIL" -eq 1 ]; then
    echo "[bench-gate-e2e] FAIL: p50 regressed >${THRESHOLD}% (cur=${CUR_P50}us ref=${REF_P50}us)"
    exit 1
fi

echo "[bench-gate-e2e] PASS: p50 within ${THRESHOLD}% of reference"
exit 0
