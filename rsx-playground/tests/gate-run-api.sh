#!/usr/bin/env bash
# gate-run-api.sh — Run gate-3 API tests and emit machine-readable failure
# report grouped by endpoint class, with 5xx regression diff vs prev run.
#
# Usage: ./gate-run-api.sh [pytest-args...]
#   e.g. ./gate-run-api.sh -x
#        ./gate-run-api.sh -k "wal"
#
# Output files (in ../tmp/):
#   gate-3-report.json       — current run result
#   gate-3-report.prev.json  — previous run (rotated automatically)
#   gate-3-diff.json         — regression diff (new failures vs prev)
#
# Exit codes:
#   0  all API tests passed
#   1  tests failed
#   2  regression detected (failures that weren't present in prev run)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLAYGROUND="$SCRIPT_DIR/.."
TMP="$PLAYGROUND/../tmp"
REPORT="$TMP/gate-3-report.json"
PREV="$TMP/gate-3-report.prev.json"
DIFF="$TMP/gate-3-diff.json"
VENV="$PLAYGROUND/.venv/bin/pytest"

mkdir -p "$TMP"

# Gate-3 API test files (excludes stress, integration, proxy)
API_TESTS=(
    tests/api_processes_test.py
    tests/api_risk_test.py
    tests/api_wal_test.py
    tests/api_logs_metrics_test.py
    tests/api_verify_test.py
    tests/api_orders_test.py
    tests/api_edge_cases_test.py
)

echo "==> [gate-3] API test suite"
echo "    report: $REPORT"
echo ""

# Run pytest — conftest.py writes gate-3-report.json on finish
cd "$PLAYGROUND"
"$VENV" "${API_TESTS[@]}" --tb=short -q "$@" || true

echo ""

# Parse report and emit summary
python3 - "$REPORT" "$PREV" "$DIFF" <<'PYEOF'
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
prev_path   = Path(sys.argv[2])
diff_path   = Path(sys.argv[3])

if not report_path.exists():
    print("ERROR: no report generated (did pytest run?)", file=sys.stderr)
    sys.exit(1)

report = json.loads(report_path.read_text())
prev   = json.loads(prev_path.read_text()) if prev_path.exists() else None

total   = report.get("total", 0)
passed  = report.get("passed", 0)
failed  = report.get("failed", 0)
skipped = report.get("skipped", 0)
by_class = report.get("by_class", {})

# ── Summary table ──────────────────────────────────────────────────
print(f"{'class':<16}  {'pass':>5}  {'fail':>5}  {'skip':>5}")
print("-" * 40)
for ec in sorted(by_class.keys()):
    g = by_class[ec]
    mark = "  FAIL" if g["failed"] else ""
    print(f"  {ec:<14}  {g['passed']:>5}  {g['failed']:>5}  {g['skipped']:>5}{mark}")
print("-" * 40)
print(f"  {'TOTAL':<14}  {passed:>5}  {failed:>5}  {skipped:>5}")
print("")

# ── Failing test IDs grouped by endpoint class ────────────────────
if failed:
    print("FAILURES by endpoint class:")
    for ec in sorted(by_class.keys()):
        g = by_class[ec]
        if not g["failures"]:
            continue
        print(f"\n  [{ec}] {len(g['failures'])} failure(s):")
        for f in g["failures"]:
            # Trim longrepr to first line for readability
            reason = f.get("reason", "").strip().splitlines()
            short  = reason[0][:120] if reason else ""
            print(f"    FAIL {f['test']}")
            if short:
                print(f"         {short}")

# ── Regression diff vs previous run ──────────────────────────────
if prev:
    prev_by_class = prev.get("by_class", {})

    # Collect current failed test IDs
    cur_failed  = set()
    prev_failed = set()

    for ec, g in by_class.items():
        for f in g.get("failures", []):
            cur_failed.add(f["test"])

    for ec, g in prev_by_class.items():
        for f in g.get("failures", []):
            prev_failed.add(f["test"])

    new_failures  = cur_failed  - prev_failed   # regressions
    fixed         = prev_failed - cur_failed     # improvements
    still_failing = cur_failed  & prev_failed    # unchanged

    diff = {
        "new_failures":  sorted(new_failures),
        "fixed":         sorted(fixed),
        "still_failing": sorted(still_failing),
        "regression":    len(new_failures) > 0,
    }
    diff_path.write_text(json.dumps(diff, indent=2))

    print("\n── Regression diff vs previous run ──────────────────")
    if new_failures:
        print(f"  NEW failures ({len(new_failures)} regressions):")
        for t in sorted(new_failures):
            print(f"    + {t}")
    if fixed:
        print(f"  Fixed ({len(fixed)}):")
        for t in sorted(fixed):
            print(f"    - {t}")
    if still_failing:
        print(f"  Still failing ({len(still_failing)}) — unchanged")
    if not new_failures and not fixed and not still_failing:
        print("  No change vs previous run (both clean)")

    if new_failures:
        print(f"\n  REGRESSION DETECTED: {len(new_failures)} new failure(s)")
        sys.exit(2)
else:
    diff_path.write_text(json.dumps({
        "new_failures": [],
        "fixed": [],
        "still_failing": [],
        "regression": False,
        "note": "no previous run to diff",
    }, indent=2))
    print("(no previous run to diff against)")

# Exit with test result
sys.exit(0 if failed == 0 else 1)
PYEOF
EOF_CODE=$?

echo ""
if [[ $EOF_CODE -eq 0 ]]; then
    echo "==> [gate-3] PASS (see $REPORT)"
elif [[ $EOF_CODE -eq 2 ]]; then
    echo "==> [gate-3] REGRESSION DETECTED (see $DIFF)"
else
    echo "==> [gate-3] FAIL (see $REPORT)"
fi

exit $EOF_CODE
